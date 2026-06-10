//! `EXPLAIN` / `PROFILE` plan rendering (slice 14): a query plan is the tree it
//! is, not a flat table. A pure prefix predicate detects a plan query, and the
//! returned operator column (indented `* Operator` strings) is parsed into a
//! navigable, collapsible outline.
//!
//! Verified against Memgraph 3.10.1: `EXPLAIN` returns one `QUERY PLAN` column;
//! `PROFILE` returns an `OPERATOR` column plus hits/time columns (annotations,
//! attached in slice 15). Both encode the tree by the operator string's leading
//! indentation, which this module parses identically.

use mgconsole_core::{lex, Record, TokenKind, Value};

/// Whether `query`'s leading lexer token is `EXPLAIN` or `PROFILE`
/// (case-insensitive) — so its result is a plan to render as a tree, not a
/// table. Pure: a `&str` predicate, no IO.
pub fn is_plan_query(query: &str) -> bool {
    lex(query)
        .into_iter()
        .find(|token| !matches!(token.kind, TokenKind::Whitespace))
        .is_some_and(|token| {
            let word = token.text(query).to_uppercase();
            word == "EXPLAIN" || word == "PROFILE"
        })
}

/// One operator line of a plan: its indentation depth, its text, and whether its
/// subtree is collapsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanLine {
    pub depth: usize,
    pub operator: String,
    pub collapsed: bool,
}

/// A parsed query plan: the operator lines in order, with a selection cursor for
/// keyboard navigation. The tree is encoded by `depth` (a line is a child of the
/// nearest preceding line of smaller depth).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Plan {
    pub lines: Vec<PlanLine>,
    pub selected: usize,
}

impl Plan {
    /// Parse the operator column (column 0) of a plan result into lines.
    pub fn parse(rows: &[Record]) -> Self {
        let lines = rows
            .iter()
            .filter_map(|row| match row.fields().first() {
                Some(Value::String(text)) => Some(parse_line(text)),
                _ => None,
            })
            .collect();
        Self { lines, selected: 0 }
    }

    /// Whether line `index` has a subtree (a following line of greater depth).
    pub fn has_children(&self, index: usize) -> bool {
        matches!(
            (self.lines.get(index), self.lines.get(index + 1)),
            (Some(node), Some(next)) if next.depth > node.depth
        )
    }

    /// The indices of the lines currently visible (descendants of a collapsed
    /// node are hidden), top to bottom.
    pub fn visible(&self) -> Vec<usize> {
        let mut out = Vec::new();
        let mut hidden_below: Option<usize> = None;
        for (index, line) in self.lines.iter().enumerate() {
            if let Some(depth) = hidden_below {
                if line.depth > depth {
                    continue;
                }
                hidden_below = None;
            }
            out.push(index);
            if line.collapsed && self.has_children(index) {
                hidden_below = Some(line.depth);
            }
        }
        out
    }

    /// Move the selection by `delta` among the visible lines (clamped).
    pub fn move_selection(&mut self, delta: isize) {
        let visible = self.visible();
        if visible.is_empty() {
            return;
        }
        let pos = visible.iter().position(|&i| i == self.selected).unwrap_or(0);
        let next = (pos as isize + delta).clamp(0, visible.len() as isize - 1) as usize;
        self.selected = visible[next];
    }

    /// Collapse or expand the selected node's subtree (if it has one).
    pub fn toggle(&mut self) {
        if self.has_children(self.selected) {
            self.lines[self.selected].collapsed = !self.lines[self.selected].collapsed;
        }
    }

    /// Collapse the selected node's subtree.
    pub fn collapse(&mut self) {
        if self.has_children(self.selected) {
            self.lines[self.selected].collapsed = true;
        }
    }

    /// Expand the selected node's subtree.
    pub fn expand(&mut self) {
        if let Some(line) = self.lines.get_mut(self.selected) {
            line.collapsed = false;
        }
    }
}

/// Parse one operator string into a [`PlanLine`]: leading spaces are the depth,
/// the rest (after the `*` bullet) is the operator text.
fn parse_line(text: &str) -> PlanLine {
    let depth = text.chars().take_while(|c| *c == ' ').count();
    let operator = text.trim_start().trim_start_matches('*').trim().to_string();
    PlanLine {
        depth,
        operator,
        collapsed: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_explain_and_profile_case_insensitively() {
        assert!(is_plan_query("EXPLAIN MATCH (n) RETURN n"));
        assert!(is_plan_query("  profile MATCH (n) RETURN n"));
        assert!(is_plan_query("Explain RETURN 1"));
        assert!(!is_plan_query("MATCH (n) RETURN n"));
        assert!(!is_plan_query("RETURN 'EXPLAIN'")); // a string, not the leading word
        assert!(!is_plan_query(""));
    }

    fn plan_row(text: &str) -> Record {
        Record::new(vec![Value::String(text.to_string())])
    }

    #[test]
    fn parses_operator_lines_with_depth() {
        let rows = vec![plan_row(" * Produce {n}"), plan_row(" * ScanAll (n)")];
        let plan = Plan::parse(&rows);
        assert_eq!(plan.lines.len(), 2);
        assert_eq!(plan.lines[0].operator, "Produce {n}");
        assert_eq!(plan.lines[0].depth, 1);
    }

    #[test]
    fn collapsing_a_node_hides_its_deeper_descendants() {
        // depth 0 root with two deeper children, then a depth-0 sibling.
        let plan = Plan {
            lines: vec![
                PlanLine { depth: 0, operator: "Root".into(), collapsed: false },
                PlanLine { depth: 2, operator: "ChildA".into(), collapsed: false },
                PlanLine { depth: 2, operator: "ChildB".into(), collapsed: false },
                PlanLine { depth: 0, operator: "Sibling".into(), collapsed: false },
            ],
            selected: 0,
        };
        assert_eq!(plan.visible(), vec![0, 1, 2, 3], "all visible expanded");
        let mut collapsed = plan.clone();
        collapsed.toggle(); // collapse Root (selected 0, has children)
        assert_eq!(collapsed.visible(), vec![0, 3], "children hidden, sibling stays");
    }

    #[test]
    fn navigation_skips_hidden_lines() {
        let mut plan = Plan {
            lines: vec![
                PlanLine { depth: 0, operator: "Root".into(), collapsed: true },
                PlanLine { depth: 2, operator: "Child".into(), collapsed: false },
                PlanLine { depth: 0, operator: "Sibling".into(), collapsed: false },
            ],
            selected: 0,
        };
        // Root is collapsed, so Down jumps over the hidden Child to Sibling.
        plan.move_selection(1);
        assert_eq!(plan.selected, 2);
    }

    #[test]
    fn toggle_does_nothing_on_a_leaf() {
        let mut plan = Plan {
            lines: vec![PlanLine { depth: 0, operator: "Leaf".into(), collapsed: false }],
            selected: 0,
        };
        plan.toggle();
        assert!(!plan.lines[0].collapsed, "a leaf cannot collapse");
    }
}
