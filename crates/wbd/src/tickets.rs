//! Tickets: Azure DevOps work items, through dova. Listing, reading,
//! commenting and filing are dova's own commands. What a ticket's type
//! has, its parts, its fields, its states and what goes under it, is
//! asked of Azure DevOps with `dova api`, once for each type, and kept.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex};

use protocol::{
    Child, Comment, Field, FieldKind, Part, PartEdit, Ticket, TicketRow, TicketState, WorkItemRef,
};
use serde_json::{Value, json};
use tokio::process::Command;

use crate::dova::Dova;
use crate::error::Error;
use crate::pulls;
use crate::review::Reviews;

/// A ticket in one of these states is finished with, whatever its type calls them.
const OVER: [&str; 3] = ["Closed", "Done", "Removed"];

/// Fields the tab shows in places of their own, whatever a form says.
const ELSEWHERE: [&str; 6] = [
    "System.Title",
    "System.State",
    "System.Reason",
    "System.AssignedTo",
    "System.IterationPath",
    "System.AreaPath",
];

/// A ticket type: what Azure DevOps calls it inside, its colour and its states.
struct Kind {
    reference: String,
    color: Option<String>,
    states: Vec<TicketState>,
}

/// What a ticket type's form has: the parts of its text, and its fields.
#[derive(Debug, Default, PartialEq)]
struct Form {
    /// Each part's field and label.
    parts: Vec<(String, String)>,
    fields: Vec<FormField>,
    /// Every field of the type by the name people see, to find a part dova names.
    named: HashMap<String, String>,
}

#[derive(Debug, PartialEq)]
struct FormField {
    field: String,
    label: String,
    kind: FieldKind,
    options: Vec<String>,
}

/// A project's backlog levels, from the top: epics, features, stories, tasks.
#[derive(Debug, Default, PartialEq)]
struct Levels {
    /// Each level's types, and the type a new ticket on it gets.
    levels: Vec<(Vec<String>, String)>,
    /// The types on the lowest level: what is ticked off.
    tasks: Vec<String>,
    /// The types that are bugs. Where they sit is a setting, and tasks go under them either way.
    bugs: Vec<String>,
}

impl Levels {
    /// The type that goes under a ticket of this type.
    fn child_of(&self, kind: &str) -> Option<String> {
        let same = |name: &String| name.eq_ignore_ascii_case(kind);
        let last = self.levels.len().checked_sub(1)?;
        let level = self
            .levels
            .iter()
            .position(|(types, _)| types.iter().any(same));
        match level {
            Some(level) if level < last => Some(self.levels[level + 1].1.clone()),
            Some(_) if self.bugs.iter().any(same) && !self.tasks.iter().any(same) => {
                Some(self.levels[last].1.clone())
            }
            Some(_) => None,
            None if self.bugs.iter().any(same) => Some(self.levels[last].1.clone()),
            None => None,
        }
    }
}

/// What Azure DevOps says about ticket types and projects, asked once and kept while wbd runs.
#[derive(Default)]
pub struct Forms {
    kinds: Mutex<HashMap<(String, String), Arc<Kind>>>,
    forms: Mutex<HashMap<(String, String), Arc<Form>>>,
    levels: Mutex<HashMap<String, Arc<Levels>>>,
    iterations: Mutex<HashMap<String, Arc<Vec<String>>>>,
}

impl Forms {
    async fn kind(
        &self,
        dova: &Dova,
        dir: &str,
        project: &str,
        kind: &str,
    ) -> Result<Arc<Kind>, Error> {
        let key = (project.to_lowercase(), kind.to_lowercase());
        if let Some(known) = self.kinds.lock().expect("not poisoned").get(&key) {
            return Ok(known.clone());
        }
        let path = format!(
            "/{}/_apis/wit/workitemtypes/{}?api-version=7.1",
            segment(project),
            segment(kind)
        );
        let raw = get(dova, dir, path).await?;
        let read = Arc::new(Kind {
            reference: text(&raw["referenceName"]),
            color: raw["color"].as_str().map(str::to_string),
            states: raw["states"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|state| {
                    Some(TicketState {
                        name: state["name"].as_str()?.to_string(),
                        category: text(&state["category"]),
                    })
                })
                .collect(),
        });
        self.kinds
            .lock()
            .expect("not poisoned")
            .insert(key, read.clone());
        Ok(read)
    }

    /// A type's form. An Azure DevOps that will not say gives an empty one: the ticket still opens.
    async fn form(
        &self,
        dova: &Dova,
        dir: &str,
        project: &str,
        kind: &Kind,
        name: &str,
    ) -> Arc<Form> {
        let key = (project.to_lowercase(), name.to_lowercase());
        if let Some(known) = self.forms.lock().expect("not poisoned").get(&key) {
            return known.clone();
        }
        let asked = async {
            let about = get(
                dova,
                dir,
                format!(
                    "/_apis/projects/{}?includeCapabilities=true&api-version=7.1",
                    segment(project)
                ),
            )
            .await?;
            let process = text(&about["capabilities"]["processTemplate"]["templateTypeId"]);
            let (layout, typed, all) = tokio::join!(
                get(
                    dova,
                    dir,
                    format!(
                        "/_apis/work/processes/{process}/workitemtypes/{}?$expand=layout&api-version=7.1",
                        segment(&kind.reference)
                    ),
                ),
                get(
                    dova,
                    dir,
                    format!(
                        "/{}/_apis/wit/workitemtypes/{}/fields?$expand=allowedValues&api-version=7.1",
                        segment(project),
                        segment(name)
                    ),
                ),
                get(dova, dir, "/_apis/wit/fields?api-version=7.1".to_string()),
            );
            Ok::<Form, Error>(read_form(&layout?, &typed?, &all?))
        };
        let Ok(form) = asked.await else {
            // Not kept: it is asked again the next time the ticket opens.
            return Arc::new(Form::default());
        };
        let form = Arc::new(form);
        self.forms
            .lock()
            .expect("not poisoned")
            .insert(key, form.clone());
        form
    }

    async fn levels(&self, dova: &Dova, dir: &str, project: &str) -> Arc<Levels> {
        let key = project.to_lowercase();
        if let Some(known) = self.levels.lock().expect("not poisoned").get(&key) {
            return known.clone();
        }
        let path = format!(
            "/{}/_apis/work/backlogconfiguration?api-version=7.1",
            segment(project)
        );
        let Ok(raw) = get(dova, dir, path).await else {
            return Arc::new(Levels::default());
        };
        let levels = Arc::new(read_levels(&raw));
        self.levels
            .lock()
            .expect("not poisoned")
            .insert(key, levels.clone());
        levels
    }

    /// A project's iterations, as the paths a ticket is given.
    async fn iterations(&self, dova: &Dova, dir: &str, project: &str) -> Arc<Vec<String>> {
        let key = project.to_lowercase();
        if let Some(known) = self.iterations.lock().expect("not poisoned").get(&key) {
            return known.clone();
        }
        let path = format!(
            "/{}/_apis/wit/classificationnodes/iterations?$depth=8&api-version=7.1",
            segment(project)
        );
        let Ok(raw) = get(dova, dir, path).await else {
            return Arc::new(Vec::new());
        };
        let mut paths = Vec::new();
        iteration_paths(&raw, "", &mut paths);
        let paths = Arc::new(paths);
        self.iterations
            .lock()
            .expect("not poisoned")
            .insert(key, paths.clone());
        paths
    }
}

/// A type's form, from its layout, its fields with what each can be set to, and every field's kind.
fn read_form(layout: &Value, typed: &Value, all: &Value) -> Form {
    let listed = |value: &Value| value["value"].as_array().cloned().unwrap_or_default();
    let kinds: HashMap<String, (String, bool)> = listed(all)
        .iter()
        .map(|field| {
            (
                text(&field["referenceName"]),
                (
                    text(&field["type"]),
                    field["readOnly"].as_bool() == Some(true),
                ),
            )
        })
        .collect();
    let mut allowed = HashMap::new();
    let mut names = HashMap::new();
    let mut named = HashMap::new();
    for field in listed(typed) {
        let reference = text(&field["referenceName"]);
        let options: Vec<String> = field["allowedValues"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|value| {
                value
                    .as_str()
                    .map_or_else(|| value.to_string(), str::to_string)
            })
            .collect();
        allowed.insert(reference.clone(), options);
        names.insert(reference.clone(), text(&field["name"]));
        named.insert(plain(&text(&field["name"])), reference);
    }

    // A ticket can hold a part its type does not have, left from when it was another type.
    // Its field is found among every field there is.
    for field in listed(all) {
        named
            .entry(plain(&text(&field["name"])))
            .or_insert_with(|| text(&field["referenceName"]));
    }
    let mut form = Form {
        named,
        ..Form::default()
    };
    let mut seen = HashSet::new();
    let shown = |thing: &Value| thing["visible"].as_bool() != Some(false);
    let each = |value: &Value| value.as_array().cloned().unwrap_or_default();
    for page in each(&layout["layout"]["pages"]) {
        if page["pageType"]
            .as_str()
            .is_some_and(|kind| kind != "custom")
            || !shown(&page)
        {
            continue;
        }
        for section in each(&page["sections"]) {
            for group in each(&section["groups"]).iter().filter(|group| shown(group)) {
                for control in each(&group["controls"])
                    .iter()
                    .filter(|control| shown(control))
                {
                    let field = text(&control["id"]);
                    // A control that is no field, such as the list of links, has no kind.
                    let Some((kind, locked)) = kinds.get(&field) else {
                        continue;
                    };
                    if ELSEWHERE.contains(&field.as_str()) || !seen.insert(field.clone()) {
                        continue;
                    }
                    let label = [&control["label"], &group["label"]]
                        .iter()
                        .filter_map(|label| label.as_str())
                        .find(|label| !label.is_empty())
                        .map_or_else(
                            || names.get(&field).cloned().unwrap_or_default(),
                            str::to_string,
                        );
                    if kind == "html" || control["controlType"] == "HtmlFieldControl" {
                        form.parts.push((field, label));
                        continue;
                    }
                    if *locked || control["readOnly"].as_bool() == Some(true) {
                        continue;
                    }
                    let options = allowed.get(&field).cloned().unwrap_or_default();
                    let kind = match kind.as_str() {
                        _ if !options.is_empty() => FieldKind::List,
                        "integer" | "double" => FieldKind::Number,
                        "dateTime" => FieldKind::Date,
                        "boolean" => FieldKind::List,
                        // Someone other than who it is assigned to, or a place in a tree: not changed from here.
                        "identity" | "treePath" | "history" => continue,
                        _ => FieldKind::Text,
                    };
                    let options = match (kind, options.is_empty()) {
                        (FieldKind::List, true) => vec!["True".to_string(), "False".to_string()],
                        _ => options,
                    };
                    form.fields.push(FormField {
                        field,
                        label,
                        kind,
                        options,
                    });
                }
            }
        }
    }
    form
}

/// A project's backlog levels, from its backlog configuration.
fn read_levels(raw: &Value) -> Levels {
    let types = |level: &Value| -> Vec<String> {
        level["workItemTypes"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|kind| text(&kind["name"]))
            .collect()
    };
    let first = |level: &Value| text(&level["defaultWorkItemType"]["name"]);
    // The portfolio levels come in no order: the highest rank is the top.
    let mut portfolio: Vec<&Value> = raw["portfolioBacklogs"]
        .as_array()
        .into_iter()
        .flatten()
        .collect();
    portfolio.sort_by_key(|level| std::cmp::Reverse(level["rank"].as_i64().unwrap_or(0)));
    let mut levels: Vec<(Vec<String>, String)> = portfolio
        .iter()
        .map(|level| (types(level), first(level)))
        .collect();
    let mut stories = types(&raw["requirementBacklog"]);
    let bugs = types(&raw["bugWorkItems"]);
    if raw["bugsBehavior"] == "asRequirements" {
        stories.extend(bugs.iter().cloned());
    }
    levels.push((stories, first(&raw["requirementBacklog"])));
    let tasks = types(&raw["taskBacklog"]);
    levels.push((tasks.clone(), first(&raw["taskBacklog"])));
    levels.retain(|(_, first)| !first.is_empty());
    Levels {
        levels,
        tasks,
        bugs,
    }
}

/// The iterations under a node, as the paths a ticket is given: `Storefront\Iteration 1`.
fn iteration_paths(node: &Value, above: &str, out: &mut Vec<String>) {
    let Some(name) = node["name"].as_str() else {
        return;
    };
    let path = match above {
        "" => name.to_string(),
        above => format!("{above}\\{name}"),
    };
    out.push(path.clone());
    for child in node["children"].as_array().into_iter().flatten() {
        iteration_paths(child, &path, out);
    }
}

/// Your tickets that are not finished, or with `words` every ticket whose
/// title has them, across every project a folder of yours is cloned from: your
/// workspaces, and the clones in your repos folder.
pub async fn list(
    dova: &Dova,
    clones: &[pulls::Clone],
    words: &str,
) -> Result<Vec<TicketRow>, Error> {
    let mut rows = Vec::new();
    let mut asked = HashSet::new();
    for clone in clones {
        // dova searches a whole project at once, so each project is asked once.
        if !asked.insert(clone.project.to_lowercase()) {
            continue;
        }
        let params = match words.trim() {
            "" => json!({ "assignee": "me" }),
            words => json!({ "query": words }),
        };
        let found = dova.call("wi.search", &clone.path, params).await?;
        let items: Vec<WorkItemRef> = shape(found["items"].clone())?;
        rows.extend(
            items
                .into_iter()
                .filter(|item| !words.trim().is_empty() || !over(item.state.as_deref()))
                .map(|item| TicketRow {
                    item,
                    workspace: clone.key.clone(),
                }),
        );
    }
    Ok(rows)
}

/// A ticket, with what its type's form has, what it is linked to, and what is above and under it.
pub async fn open(
    dova: &Dova,
    reviews: &Reviews,
    forms: &Forms,
    dir: &str,
    id: u64,
) -> Result<Ticket, Error> {
    let asked = json!({ "idOrUrl": id.to_string(), "full": true, "inlineImages": true });
    let (view, raw, me) = tokio::join!(
        dova.call("wi.view", dir, asked),
        get(
            dova,
            dir,
            format!("_apis/wit/workitems/{id}?$expand=relations&api-version=7.1")
        ),
        reviews.me(dova, dir),
    );
    let (view, raw, me) = (view?, raw?, me?);
    let said = |key: &str| view[key].as_str().map(str::to_string);
    let (name, project) = (
        said("type").unwrap_or_default(),
        said("project").unwrap_or_default(),
    );
    // A type is asked about in its own project, which need not be the folder's.
    let kind = forms.kind(dova, dir, &project, &name).await?;
    let (form, levels, iterations) = tokio::join!(
        forms.form(dova, dir, &project, &kind, &name),
        forms.levels(dova, dir, &project),
        forms.iterations(dova, dir, &project),
    );

    // The parts the form has, each with what dova read of it. What was written in a part the
    // form does not show comes after them: nothing that was written is left out.
    let written: Vec<(String, String)> = view["body"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|section| (text(&section["label"]), text(&section["text"])))
        .collect();
    let mut parts: Vec<Part> = form
        .parts
        .iter()
        .map(|(field, label)| Part {
            field: field.clone(),
            label: label.clone(),
            text: String::new(),
            shown: None,
            source: None,
            markdown: false,
        })
        .collect();
    for (label, words) in written {
        let field = form.named.get(&plain(&label)).cloned();
        let at = parts.iter().position(|part| {
            Some(&part.field) == field.as_ref() || plain(&part.label) == plain(&label)
        });
        match at {
            Some(at) => parts[at].text = words,
            None => parts.push(Part {
                field: field.unwrap_or_default(),
                label,
                text: words,
                shown: None,
                source: None,
                markdown: false,
            }),
        }
    }
    // Each part as Azure DevOps keeps it, so its formatting is drawn and kept when it is changed.
    for part in parts.iter_mut().filter(|part| !part.field.is_empty()) {
        part.source = raw["fields"][&part.field].as_str().map(str::to_string);
        part.markdown = raw["multilineFieldsFormat"][&part.field]
            .as_str()
            .is_some_and(|format| format.eq_ignore_ascii_case("markdown"));
        part.shown = part.source.clone();
    }
    // A picture in a part is only given to someone signed in. dova puts each one where its link was.
    let pictured = |part: &Part| {
        part.source
            .as_deref()
            .is_some_and(|html| html.contains("<img"))
    };
    if parts.iter().any(pictured) {
        let asked = json!({ "path": format!("_apis/wit/workitems/{id}?api-version=7.1"), "inlineImages": true });
        if let Ok(inlined) = dova.call("api", dir, asked).await {
            for part in parts.iter_mut().filter(|part| part.source.is_some()) {
                if let Some(html) = inlined["fields"][&part.field].as_str() {
                    part.shown = Some(html.to_string());
                }
            }
        }
    }

    let assigned = &raw["fields"]["System.AssignedTo"];
    let mut fields = vec![
        Field {
            field: "System.AssignedTo".into(),
            label: "Assigned".into(),
            value: said("assignedTo"),
            kind: FieldKind::Person,
            options: Vec::new(),
        },
        Field {
            field: "System.IterationPath".into(),
            label: "Iteration".into(),
            value: said("iterationPath"),
            kind: FieldKind::List,
            options: iterations.to_vec(),
        },
    ];
    fields.extend(form.fields.iter().map(|field| Field {
        field: field.field.clone(),
        label: field.label.clone(),
        value: shown(&raw["fields"][&field.field], field.kind),
        kind: field.kind,
        options: field.options.clone(),
    }));

    let mut children = Vec::new();
    for item in shape::<Vec<WorkItemRef>>(view["children"].clone())? {
        let kind_name = item.kind.clone().unwrap_or_default();
        // A child's states are its own type's. A type that can't be asked about counts as not done.
        let states = forms.kind(dova, dir, &project, &kind_name).await.ok();
        let done = states.is_some_and(|kind| {
            kind.states.iter().any(|state| {
                Some(&state.name) == item.state.as_ref() && state.category == "Completed"
            })
        });
        let task = levels
            .tasks
            .iter()
            .any(|name| name.eq_ignore_ascii_case(&kind_name));
        children.push(Child { item, done, task });
    }

    let (pulls, branches) = linked(&raw);
    Ok(Ticket {
        id,
        title: said("title").unwrap_or_default(),
        state: said("state").unwrap_or_default(),
        color: kind.color.clone(),
        assigned_to: said("assignedTo"),
        mine: assigned["id"].as_str() == Some(me.as_str()),
        parts,
        fields,
        url: said("url").unwrap_or_default(),
        parent: shape::<Option<WorkItemRef>>(view["parent"].clone())?,
        children,
        child_kind: levels.child_of(&name),
        comments: shape::<Vec<Comment>>(view["comments"].clone())?,
        pulls,
        branches,
        states: kind.states.clone(),
        kind: name,
        project,
    })
}

/// A field's value as the tab shows it.
fn shown(value: &Value, kind: FieldKind) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(words) if kind == FieldKind::Date => Some(words.chars().take(10).collect()),
        Value::String(words) => Some(words.clone()),
        Value::Number(number) => Some(match number.as_f64() {
            Some(whole) if whole.fract() == 0.0 => format!("{whole:.0}"),
            _ => number.to_string(),
        }),
        Value::Bool(yes) => Some(if *yes { "True" } else { "False" }.to_string()),
        other => other["displayName"].as_str().map(str::to_string),
    }
}

/// Moves a ticket to a state, or assigns it to you or to nobody.
pub async fn set(
    dova: &Dova,
    dir: &str,
    id: u64,
    state: Option<&str>,
    mine: Option<bool>,
) -> Result<(), Error> {
    let mut params = json!({ "idOrUrl": id.to_string() });
    if let Some(state) = state {
        params["state"] = json!(state);
    }
    if let Some(mine) = mine {
        params["assignee"] = json!(if mine { "me" } else { "none" });
    }
    dova.call("wi.edit", dir, params).await.map(|_| ())
}

/// Changes a ticket's title, parts of its text, or both.
pub async fn edit(
    dova: &Dova,
    dir: &str,
    id: u64,
    title: Option<String>,
    parts: Vec<PartEdit>,
) -> Result<(), Error> {
    let mut changes = Vec::new();
    if let Some(title) = title {
        changes.push(json!({ "op": "add", "path": "/fields/System.Title", "value": title }));
    }
    if !parts.is_empty() {
        // A part is kept as HTML, unless it was switched to Markdown in Azure DevOps.
        let raw = get(
            dova,
            dir,
            format!("_apis/wit/workitems/{id}?api-version=7.1"),
        )
        .await?;
        for part in parts {
            let field = field_name(&part.field)?;
            let markdown = raw["multilineFieldsFormat"][field]
                .as_str()
                .is_some_and(|format| format.eq_ignore_ascii_case("markdown"));
            let value = if markdown { part.text } else { part.html };
            changes
                .push(json!({ "op": "add", "path": format!("/fields/{field}"), "value": value }));
        }
    }
    patch(dova, dir, id, changes).await
}

/// Sets one field of a ticket. With no value, the field is emptied.
pub async fn set_field(
    dova: &Dova,
    dir: &str,
    id: u64,
    field: &str,
    value: Option<String>,
) -> Result<(), Error> {
    let path = format!("/fields/{}", field_name(field)?);
    let change = match value.filter(|value| !value.trim().is_empty()) {
        Some(value) => json!({ "op": "add", "path": path, "value": value.trim() }),
        None => json!({ "op": "remove", "path": path }),
    };
    patch(dova, dir, id, vec![change]).await
}

/// Files a ticket under another one, of the type that goes under it. Its number is the answer.
pub async fn child(
    dova: &Dova,
    reviews: &Reviews,
    forms: &Forms,
    dir: &str,
    id: u64,
    title: &str,
) -> Result<u64, Error> {
    let above = open(dova, reviews, forms, dir, id).await?;
    let kind = above.child_kind.ok_or_else(|| {
        Error::Conflict(format!(
            "Nothing goes under a {} in this project.",
            above.kind
        ))
    })?;
    // It goes where the ticket above it is: the same area and iteration. dova need not ask which team.
    let params =
        json!({ "title": title, "type": kind, "parent": id.to_string(), "like": id.to_string() });
    let made = dova.call("wi.create", dir, params).await?;
    made["id"]
        .as_u64()
        .ok_or_else(|| Error::Dova("dova didn't say which ticket it filed".into()))
}

/// Takes a ticket out from under another one. Neither is deleted.
pub async fn unlink(dova: &Dova, dir: &str, id: u64, child: u64) -> Result<(), Error> {
    let raw = get(
        dova,
        dir,
        format!("_apis/wit/workitems/{id}?$expand=relations&api-version=7.1"),
    )
    .await?;
    let end = format!("/{child}");
    let at = raw["relations"]
        .as_array()
        .into_iter()
        .flatten()
        .position(|relation| {
            relation["rel"] == "System.LinkTypes.Hierarchy-Forward"
                && relation["url"]
                    .as_str()
                    .is_some_and(|url| url.ends_with(&end))
        })
        .ok_or_else(|| Error::NotFound(format!("#{child} is not under #{id} any more.")))?;
    // The place of a link in the list is only right while nothing else changed the ticket.
    let changes = vec![
        json!({ "op": "test", "path": "/rev", "value": raw["rev"] }),
        json!({ "op": "remove", "path": format!("/relations/{at}") }),
    ];
    patch(dova, dir, id, changes).await
}

/// Ticks a ticket off, or takes the tick back: it moves to the state its type
/// has for finished, or back to the one for being worked on.
pub async fn done(dova: &Dova, forms: &Forms, dir: &str, id: u64, done: bool) -> Result<(), Error> {
    let raw = get(
        dova,
        dir,
        format!("_apis/wit/workitems/{id}?api-version=7.1"),
    )
    .await?;
    let (project, name) = (
        text(&raw["fields"]["System.TeamProject"]),
        text(&raw["fields"]["System.WorkItemType"]),
    );
    let kind = forms.kind(dova, dir, &project, &name).await?;
    let first = |category: &str| kind.states.iter().find(|state| state.category == category);
    let to = match done {
        true => first("Completed"),
        false => first("InProgress").or_else(|| first("Proposed")),
    }
    .ok_or_else(|| Error::Conflict(format!("A {name} has no state for that.")))?;
    set(dova, dir, id, Some(&to.name), None).await
}

pub async fn comment(dova: &Dova, dir: &str, id: u64, words: &str) -> Result<(), Error> {
    let params = json!({ "idOrUrl": id.to_string(), "text": words });
    dova.call("wi.comment", dir, params).await.map(|_| ())
}

/// Files a ticket in the project `dir` is cloned from. Its number is the answer.
pub async fn create(
    dova: &Dova,
    dir: &str,
    title: &str,
    kind: &str,
    team: Option<&str>,
) -> Result<u64, Error> {
    let mut params = json!({ "title": title, "type": kind });
    if let Some(team) = team {
        params["team"] = json!(team);
    }
    let made = dova.call("wi.create", dir, params).await?;
    made["id"]
        .as_u64()
        .ok_or_else(|| Error::Dova("dova didn't say which ticket it filed".into()))
}

/// Starts work on a ticket in the folder `dir`: a new branch from the
/// repo's main one, linked to the ticket, and the ticket assigned to you.
/// The branch's name is the answer. A folder with changes that are not
/// committed is left alone.
pub async fn start(
    dova: &Dova,
    reviews: &Reviews,
    forms: &Forms,
    dir: &str,
    id: u64,
) -> Result<String, Error> {
    let ticket = open(dova, reviews, forms, dir, id).await?;
    let folder = Path::new(dir);
    let changes = run(folder, &["status", "--porcelain"])
        .await
        .map_err(Error::Io)?;
    if !changes.trim().is_empty() {
        return Err(Error::Conflict(
            "This folder has changes that are not committed. Commit them or put them aside first."
                .into(),
        ));
    }
    let main = run(
        folder,
        &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
    )
    .await
    .map_or("origin/main".to_string(), |head| head.trim().to_string());
    // Start from where the main branch is now, if it can be asked.
    let name = main.strip_prefix("origin/").unwrap_or(&main).to_string();
    let _ = run(folder, &["fetch", "--quiet", "--no-tags", "origin", &name]).await;
    let branch = branch_name(&ticket.kind, id, &ticket.title);
    run(
        folder,
        &["switch", "--quiet", "--no-track", "-c", &branch, &main],
    )
    .await
    .map_err(|why| Error::Io(format!("couldn't make the branch {branch}: {why}")))?;
    dova.call("link", dir, json!({ "ids": [id.to_string()] }))
        .await?;
    if !ticket.mine {
        set(dova, dir, id, None, Some(true)).await?;
    }
    Ok(branch)
}

/// What the agent is told about a ticket before anything you write.
pub async fn brief(
    dova: &Dova,
    reviews: &Reviews,
    forms: &Forms,
    dir: &str,
    id: u64,
) -> Result<String, Error> {
    let ticket = open(dova, reviews, forms, dir, id).await?;
    let mut out = format!(
        "You are helping someone with ticket #{id} \"{title}\", a {kind} in the {project} project on Azure DevOps. It is {state}.\n\
         This folder is the repo the ticket is about, as the person has it checked out. Read the code before you answer about it.\n\
         dova is installed. `dova wi view {id}` reads the ticket as it is now, `dova wi search` finds others, and `dova status` says where this branch stands.\n\
         Do not change the ticket, comment on it, or edit files unless you are asked to: the person does that from the Workbench.\n\
         When you are asked to draft a comment, answer with only the text to post.\n\
         A message may start with an `<about>` block. It says what the person is looking at as they ask. It is not part of what they wrote.\n",
        title = ticket.title,
        kind = ticket.kind,
        project = ticket.project,
        state = ticket.state,
    );
    for part in ticket
        .parts
        .iter()
        .filter(|part| !part.text.trim().is_empty())
    {
        out += &format!("\n## {}\n{}\n", part.label, clip(&part.text, 6_000));
    }
    let filled: Vec<String> = ticket
        .fields
        .iter()
        .filter_map(|field| Some(format!("- {}: {}", field.label, field.value.as_ref()?)))
        .collect();
    if !filled.is_empty() {
        out += &format!("\n## Fields\n{}\n", filled.join("\n"));
    }
    if let Some(parent) = &ticket.parent {
        out += &format!(
            "\n## Parent\n- #{} {}\n",
            parent.id,
            parent.title.as_deref().unwrap_or("")
        );
    }
    if !ticket.children.is_empty() {
        out += "\n## Children\n";
        for child in &ticket.children {
            let (title, state) = (
                child.item.title.as_deref().unwrap_or(""),
                child.item.state.as_deref().unwrap_or(""),
            );
            out += &format!("- #{} {title} ({state})\n", child.item.id);
        }
    }
    if !ticket.pulls.is_empty() || !ticket.branches.is_empty() {
        out += "\n## Linked work\n";
        for pull in &ticket.pulls {
            out += &format!("- pull request #{pull} (`dova pr view {pull}`)\n");
        }
        for branch in &ticket.branches {
            out += &format!("- branch {branch}\n");
        }
    }
    if !ticket.comments.is_empty() {
        out += "\n## Discussion\n";
        for comment in &ticket.comments {
            let author = comment.author.as_deref().unwrap_or("Someone");
            out += &format!(
                "- {author}: {}\n",
                clip(&comment.text, 1_500).replace('\n', " ")
            );
        }
    }
    Ok(clip(&out, 60_000))
}

async fn get(dova: &Dova, dir: &str, path: String) -> Result<Value, Error> {
    dova.call("api", dir, json!({ "path": path })).await
}

/// Changes a work item the way Azure DevOps takes changes: a list of them, each naming its place.
async fn patch(dova: &Dova, dir: &str, id: u64, changes: Vec<Value>) -> Result<(), Error> {
    if changes.is_empty() {
        return Ok(());
    }
    let params = json!({
        "path": format!("_apis/wit/workitems/{id}?api-version=7.1"),
        "method": "PATCH",
        "body": Value::Array(changes).to_string(),
        "contentType": "application/json-patch+json",
    });
    dova.call("api", dir, params).await.map(|_| ())
}

/// A field's name is put in a path, so it must be nothing but a name.
fn field_name(field: &str) -> Result<&str, Error> {
    let plain = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-');
    match !field.is_empty() && field.chars().all(plain) {
        true => Ok(field),
        false => Err(Error::NotFound(format!(
            "\"{field}\" is no field of a ticket."
        ))),
    }
}

fn shape<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, Error> {
    serde_json::from_value(value)
        .map_err(|err| Error::Dova(format!("couldn't read what dova said: {err}")))
}

fn text(value: &Value) -> String {
    value.as_str().unwrap_or_default().to_string()
}

/// A label with only its letters and digits, in lower case: `Repro Steps` and `repro steps` are the same part.
fn plain(label: &str) -> String {
    label
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

fn over(state: Option<&str>) -> bool {
    state.is_some_and(|state| OVER.contains(&state))
}

fn clip(words: &str, most: usize) -> String {
    match words.char_indices().nth(most) {
        Some((cut, _)) => format!("{} […]", &words[..cut]),
        None => words.to_string(),
    }
}

/// A name as one part of a URL's path.
fn segment(name: &str) -> String {
    name.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => {
                (byte as char).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect()
}

/// The pull requests and branches a work item is linked to, from its relations.
/// Azure DevOps names them as `vstfs:///Git/PullRequestId/<project>%2F<repo>%2F<number>`
/// and `vstfs:///Git/Ref/<project>%2F<repo>%2FGB<branch>`.
fn linked(raw: &Value) -> (Vec<u64>, Vec<String>) {
    let (mut pulls, mut branches) = (Vec::new(), Vec::new());
    for relation in raw["relations"].as_array().into_iter().flatten() {
        let Some(url) = relation["url"].as_str() else {
            continue;
        };
        // The name is three parts joined by an encoded slash, and a branch has slashes of its own.
        let name = url
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .replace("%2f", "%2F");
        let Some(last) = name.splitn(3, "%2F").nth(2) else {
            continue;
        };
        if url.contains("/Git/PullRequestId/") {
            pulls.extend(last.parse::<u64>().ok());
        } else if url.contains("/Git/Ref/") {
            branches.extend(
                last.strip_prefix("GB")
                    .map(|branch| branch.replace("%2F", "/")),
            );
        }
    }
    pulls.sort_unstable();
    (pulls, branches)
}

/// `fix/5219-null-check-missing` for a bug, `feature/…` for anything else:
/// the ticket's number and the first words of its title.
fn branch_name(kind: &str, id: u64, title: &str) -> String {
    let words: Vec<String> = title
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .take(4)
        .map(str::to_lowercase)
        .collect();
    let prefix = if kind.eq_ignore_ascii_case("bug") {
        "fix"
    } else {
        "feature"
    };
    match words.is_empty() {
        true => format!("{prefix}/{id}"),
        false => format!("{prefix}/{id}-{}", words.join("-")),
    }
}

/// Runs git in `dir`, giving what it printed, or why it failed.
async fn run(dir: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|err| format!("couldn't run git: {err}"))?;
    match output.status.success() {
        true => Ok(String::from_utf8_lossy(&output.stdout).to_string()),
        false => Err(String::from_utf8_lossy(&output.stderr).trim().to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_a_branch_after_its_ticket() {
        assert_eq!(
            branch_name("Bug", 5219, "Null check missing on empty session token"),
            "fix/5219-null-check-missing-on"
        );
        assert_eq!(
            branch_name("Task", 6, "Add retry to login"),
            "feature/6-add-retry-to-login"
        );
        assert_eq!(branch_name("Task", 7, "¿?"), "feature/7");
    }

    #[test]
    fn reads_what_a_ticket_is_linked_to() {
        let raw = json!({ "relations": [
            { "rel": "ArtifactLink", "url": "vstfs:///Git/PullRequestId/9cae%2f378a%2f8" },
            { "rel": "ArtifactLink", "url": "vstfs:///Git/PullRequestId/9cae%2F378a%2F1" },
            { "rel": "ArtifactLink", "url": "vstfs:///Git/Ref/9cae%2F378a%2FGBfix%2F1-login-redirect" },
            { "rel": "System.LinkTypes.Hierarchy-Reverse", "url": "https://dev.azure.com/o/_apis/wit/workItems/4" },
        ] });
        let (pulls, branches) = linked(&raw);
        assert_eq!(pulls, [1, 8]);
        assert_eq!(branches, ["fix/1-login-redirect"]);
    }

    #[test]
    fn reads_a_types_form() {
        let control = |id: &str, label: &str, kind: &str| json!({ "id": id, "label": label, "controlType": kind, "visible": true });
        let layout = json!({ "layout": { "pages": [
            { "pageType": "custom", "sections": [
                { "groups": [
                    { "label": "Repro Steps", "visible": true, "controls": [control("Microsoft.VSTS.TCM.ReproSteps", "", "HtmlFieldControl")] },
                    { "label": "Planning", "visible": true, "controls": [
                        control("Microsoft.VSTS.Common.Priority", "Priority", "FieldControl"),
                        control("Microsoft.VSTS.Scheduling.StoryPoints", "Story Points", "FieldControl"),
                        control("Microsoft.VSTS.Scheduling.TargetDate", "Target", "DateTimeControl"),
                        control("System.AssignedTo", "Assigned To", "FieldControl"),
                        control("System.BoardColumn", "Column", "FieldControl"),
                    ] },
                    { "label": "Related Work", "visible": true, "controls": [control("Related Work", "", "LinksControl")] },
                    { "label": "Hidden", "visible": false, "controls": [control("Microsoft.VSTS.Common.Severity", "Severity", "FieldControl")] },
                ] },
            ] },
            { "pageType": "history", "sections": [] },
        ] } });
        let typed = json!({ "value": [
            { "referenceName": "Microsoft.VSTS.Common.Priority", "name": "Priority", "allowedValues": ["1", "2"] },
            { "referenceName": "Microsoft.VSTS.TCM.ReproSteps", "name": "Repro Steps", "allowedValues": [] },
            { "referenceName": "System.Description", "name": "Description", "allowedValues": [] },
        ] });
        let all = json!({ "value": [
            { "referenceName": "Microsoft.VSTS.TCM.ReproSteps", "type": "html" },
            { "referenceName": "Microsoft.VSTS.Common.Priority", "type": "integer" },
            { "referenceName": "Microsoft.VSTS.Scheduling.StoryPoints", "type": "double" },
            { "referenceName": "Microsoft.VSTS.Scheduling.TargetDate", "type": "dateTime" },
            { "referenceName": "Microsoft.VSTS.Common.Severity", "name": "Severity", "type": "string" },
            { "referenceName": "System.AssignedTo", "type": "identity" },
            { "referenceName": "System.BoardColumn", "type": "string", "readOnly": true },
        ] });
        let form = read_form(&layout, &typed, &all);
        assert_eq!(
            form.parts,
            [(
                "Microsoft.VSTS.TCM.ReproSteps".to_string(),
                "Repro Steps".to_string()
            )]
        );
        let fields: Vec<(&str, FieldKind, usize)> = form
            .fields
            .iter()
            .map(|field| (field.label.as_str(), field.kind, field.options.len()))
            .collect();
        assert_eq!(
            fields,
            [
                ("Priority", FieldKind::List, 2),
                ("Story Points", FieldKind::Number, 0),
                ("Target", FieldKind::Date, 0)
            ]
        );
        assert_eq!(
            form.named.get("description").map(String::as_str),
            Some("System.Description")
        );
        // A field the type does not have is still found by its name.
        assert_eq!(
            form.named.get("severity").map(String::as_str),
            Some("Microsoft.VSTS.Common.Severity")
        );
    }

    #[test]
    fn knows_what_goes_under_what() {
        let level = |types: &[&str], rank: i64| json!({ "rank": rank, "workItemTypes": types.iter().map(|name| json!({ "name": name })).collect::<Vec<_>>(), "defaultWorkItemType": { "name": types[0] } });
        let raw = json!({
            "portfolioBacklogs": [level(&["Feature"], 3), level(&["Epic"], 4)],
            "requirementBacklog": level(&["User Story"], 2),
            "taskBacklog": level(&["Task"], 1),
            "bugWorkItems": { "workItemTypes": [{ "name": "Bug" }] },
            "bugsBehavior": "asTasks",
        });
        let levels = read_levels(&raw);
        assert_eq!(levels.child_of("Epic").as_deref(), Some("Feature"));
        assert_eq!(levels.child_of("Feature").as_deref(), Some("User Story"));
        assert_eq!(levels.child_of("user story").as_deref(), Some("Task"));
        assert_eq!(levels.child_of("Bug").as_deref(), Some("Task"));
        assert_eq!(levels.child_of("Task"), None);
        assert_eq!(levels.child_of("Test Case"), None);
        assert_eq!(levels.tasks, ["Task"]);
    }

    #[test]
    fn lists_iterations_as_paths() {
        let raw = json!({ "name": "Storefront", "children": [
            { "name": "Iteration 1" },
            { "name": "Release 2", "children": [{ "name": "Sprint 5" }] },
        ] });
        let mut paths = Vec::new();
        iteration_paths(&raw, "", &mut paths);
        assert_eq!(
            paths,
            [
                "Storefront",
                "Storefront\\Iteration 1",
                "Storefront\\Release 2",
                "Storefront\\Release 2\\Sprint 5"
            ]
        );
    }

    #[test]
    fn small_things() {
        assert!(over(Some("Done")) && !over(Some("Active")) && !over(None));
        assert_eq!(segment("Product Backlog Item"), "Product%20Backlog%20Item");
        assert_eq!(shown(&json!(3.0), FieldKind::Number).as_deref(), Some("3"));
        assert_eq!(
            shown(&json!(2.5), FieldKind::Number).as_deref(),
            Some("2.5")
        );
        assert_eq!(
            shown(&json!("2026-12-15T00:00:00Z"), FieldKind::Date).as_deref(),
            Some("2026-12-15")
        );
        assert_eq!(
            shown(&json!({ "displayName": "Sam Lee" }), FieldKind::Text).as_deref(),
            Some("Sam Lee")
        );
        assert!(field_name("Microsoft.VSTS.Common.Priority").is_ok() && field_name("a/b").is_err());
    }
}
