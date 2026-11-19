//! Tickets: Azure DevOps work items, as dova reports them.

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
    /// The workspace it was asked from: a folder cloned from its organization.
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
    /// `Bug`, `Task`, `Product Backlog Item` and so on.
    #[serde(rename = "type")]
    pub kind: String,
    pub state: String,
    pub project: String,
    pub assigned_to: Option<String>,
    /// It is assigned to you.
    pub mine: bool,
    pub iteration: Option<String>,
    pub points: Option<f64>,
    /// Its text, a part at a time: the description, the steps to reproduce, the acceptance criteria.
    pub body: Vec<Section>,
    /// Its page on Azure DevOps.
    pub url: String,
    pub parent: Option<WorkItemRef>,
    pub children: Vec<WorkItemRef>,
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
pub struct Section {
    pub label: String,
    /// Markdown.
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, TS)]
#[ts(export)]
pub struct TicketState {
    pub name: String,
    /// Where the state stands, in Azure DevOps' own words: `Proposed`,
    /// `InProgress`, `Resolved`, `Completed`, `Removed`.
    pub category: String,
}
