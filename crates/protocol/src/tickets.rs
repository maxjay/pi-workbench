//! Tickets: Azure DevOps work items. What a ticket has depends on its
//! type: a bug has steps to reproduce and a severity, a story has
//! acceptance criteria and points. The parts, the fields and the states
//! are the ones the type's form has in Azure DevOps.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{Comment, WorkItemRef};

/// One of your tickets, as Home lists it.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TicketRow {
    #[serde(flatten)]
    pub item: WorkItemRef,
    /// Where it was asked from: a workspace, by its id, or a clone in your repos folder, by
    /// its path. Either is a folder cloned from its organization.
    pub workspace: String,
}

/// A ticket, with everything its tab shows.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Ticket {
    #[ts(type = "number")]
    pub id: u64,
    pub title: String,
    /// `Bug`, `Task`, `User Story` and so on.
    #[serde(rename = "type")]
    pub kind: String,
    /// The colour Azure DevOps gives its type, as six hex digits.
    pub color: Option<String>,
    pub state: String,
    pub project: String,
    pub assigned_to: Option<String>,
    /// It is assigned to you.
    pub mine: bool,
    /// Its text, a part at a time, as its type's form has them: a bug's
    /// steps to reproduce, a story's acceptance criteria. A part nobody
    /// wrote is there too, empty.
    pub parts: Vec<Part>,
    /// The fields its type's form has, in the form's order.
    pub fields: Vec<Field>,
    /// Its page on Azure DevOps.
    pub url: String,
    pub parent: Option<WorkItemRef>,
    pub children: Vec<Child>,
    /// The type a new ticket under this one has: a task under a story.
    /// None when nothing goes under its type.
    pub child_kind: Option<String>,
    /// Its discussion, the oldest first.
    pub comments: Vec<Comment>,
    /// The pull requests linked to it, by number.
    #[ts(type = "Array<number>")]
    pub pulls: Vec<u64>,
    /// The branches linked to it.
    pub branches: Vec<String>,
    /// The states a ticket of its type can be in, in order.
    pub states: Vec<TicketState>,
}

/// A part of a ticket's text.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, TS)]
#[ts(export)]
pub struct Part {
    /// The field it is kept in, by Azure DevOps' own name: `System.Description`.
    pub field: String,
    pub label: String,
    /// What it says, as plain words. Empty when nobody wrote it.
    pub text: String,
    /// The part as Azure DevOps keeps it, to draw: HTML, with each picture put where its link was.
    /// Markdown instead when `markdown` is set. None when the field it is kept in is not known.
    pub shown: Option<String>,
    /// The same with its pictures as the links they are: what is changed and written back.
    pub source: Option<String>,
    /// It is kept as Markdown, not as HTML.
    pub markdown: bool,
}

/// A part of a ticket's text, as it is written back.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, TS)]
#[ts(export)]
pub struct PartEdit {
    pub field: String,
    /// What was written, as Markdown.
    pub text: String,
    /// The same, as HTML: what Azure DevOps keeps unless the part was switched to Markdown there.
    pub html: String,
}

/// One field of a ticket's form.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, TS)]
#[ts(export)]
pub struct Field {
    /// Azure DevOps' own name for it: `Microsoft.VSTS.Common.Priority`.
    pub field: String,
    pub label: String,
    /// What it holds, as text. A date is `2026-12-15`.
    pub value: Option<String>,
    pub kind: FieldKind,
    /// What it can be set to, for a list.
    pub options: Vec<String>,
}

/// How a field is changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum FieldKind {
    Text,
    Number,
    Date,
    /// One of its options.
    List,
    /// Who it is assigned to: you, or nobody.
    Person,
}

/// A ticket under another one.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Child {
    #[serde(flatten)]
    pub item: WorkItemRef,
    /// It is finished: its state is one its type counts as completed.
    pub done: bool,
    /// It is a task: something ticked off, not a ticket with a life of its own.
    pub task: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, TS)]
#[ts(export)]
pub struct TicketState {
    pub name: String,
    /// Where the state stands, in Azure DevOps' own words: `Proposed`,
    /// `InProgress`, `Resolved`, `Completed`, `Removed`.
    pub category: String,
}
