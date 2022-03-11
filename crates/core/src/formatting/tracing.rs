use super::id::IdCounter;
use super::*;
use serde::Serialize;
use std::collections::HashSet;

thread_local! {
  pub static PRINT_NODE_IDS: IdCounter = IdCounter::default();
  pub static GRAPH_NODE_IDS: IdCounter = IdCounter::default();
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TracingResult {
  pub traces: Vec<Trace>,
  pub writer_nodes: Vec<TraceWriterNode>,
  pub print_nodes: Vec<TracePrintNode>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Trace {
  /// The relative time of the trace from the start of printing in nanoseconds.
  pub nanos: u128,
  pub print_node_id: usize,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub writer_node_id: Option<usize>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TraceWriterNode {
  pub writer_node_id: usize,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub previous_node_id: Option<usize>,
  pub text: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TracePrintNode {
  pub print_node_id: usize,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub next_print_node_id: Option<usize>,
  pub print_item: TracePrintItem,
}

#[derive(Serialize)]
#[serde(tag = "kind", content = "content", rename_all = "camelCase")]
pub enum TracePrintItem {
  String(String),
  Condition(TraceCondition),
  Info(TraceInfo),
  Signal(Signal),
  /// Identifier to the print node.
  RcPath(usize),
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TraceInfo {
  pub info_id: usize,
  pub name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TraceCondition {
  pub condition_id: usize,
  pub name: String,
  pub is_stored: bool,
  #[serde(skip_serializing_if = "Option::is_none")]
  /// Identifier to the true path print node.
  pub true_path: Option<usize>,
  #[serde(skip_serializing_if = "Option::is_none")]
  /// Identifier to the false path print node.
  pub false_path: Option<usize>,
  #[serde(skip_serializing_if = "Option::is_none")]
  /// Any infos that should cause the re-evaluation of this condition.
  /// This is only done on request for performance reasons.
  pub dependent_infos: Option<Vec<usize>>,
}

/// Gets all the TracePrintNodes for analysis from the starting node.
pub fn get_trace_print_nodes(start_node: Option<PrintItemPath>) -> Vec<TracePrintNode> {
  let mut print_nodes = Vec::new();
  let mut path_stack = Vec::new();
  let mut handled_nodes = HashSet::new();

  if let Some(start_node) = start_node {
    path_stack.push(start_node);
  }

  // do not use recursion as it will easily overflow the stack
  while let Some(node) = path_stack.pop() {
    let node_id = node.get_node_id();
    if handled_nodes.contains(&node_id) {
      continue;
    }

    // get the trace print item
    let trace_print_item = match node.get_item() {
      PrintItem::String(text) => TracePrintItem::String(text.text.to_string()),
      PrintItem::Info(info) => TracePrintItem::Info(TraceInfo {
        info_id: info.get_unique_id(),
        name: info.get_name().to_string(),
      }),
      PrintItem::Condition(condition) => {
        if let Some(true_path) = condition.get_true_path() {
          path_stack.push(true_path);
        }
        if let Some(false_path) = condition.get_false_path() {
          path_stack.push(false_path);
        }
        TracePrintItem::Condition(TraceCondition {
          condition_id: condition.get_unique_id(),
          name: condition.get_name().to_string(),
          is_stored: condition.is_stored,
          dependent_infos: condition
            .dependent_infos
            .as_ref()
            .map(|infos| infos.iter().map(|i| i.get_unique_id()).collect()),
          true_path: condition.get_true_path().map(|p| p.get_node_id()),
          false_path: condition.get_false_path().map(|p| p.get_node_id()),
        })
      }
      PrintItem::Signal(signal) => TracePrintItem::Signal(signal),
      PrintItem::RcPath(path) => {
        path_stack.push(path);
        TracePrintItem::RcPath(path.get_node_id())
      }
    };

    // create and store the trace print node
    print_nodes.push(TracePrintNode {
      print_node_id: node_id,
      next_print_node_id: node.get_next().map(|n| n.get_node_id()),
      print_item: trace_print_item,
    });

    if let Some(next) = node.get_next() {
      path_stack.push(next);
    }

    handled_nodes.insert(node_id);
  }

  print_nodes
}
