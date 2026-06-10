//! Clause scanner (slice 27): a pure function from query text to the clauses
//! that matter for import ordering. Used by parser mode (28) and to enforce
//! vertices-first ordering in parallel import (31).
//!
//! It reads the Core [`lexer`](crate::lexer)'s `Word` tokens, lowercased (ADR
//! 0008 deepening, slice 07). A keyword inside a string or comment is part of a
//! String/Comment token, not a `Word`, so it is never detected; and a `Word` is
//! a whole identifier, so `created` does not match `create`. One lexical state
//! machine, shared with `parse.rs` and the highlighter. No database.

use std::collections::BTreeSet;

use crate::lexer::{lex, TokenKind};

/// A clause relevant to import ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Clause {
    Create,
    Match,
    Merge,
    IndexCreate,
    IndexDrop,
    DetachDelete,
    Delete,
    Remove,
    StorageMode,
}

/// Detect the ordering-relevant clauses present in a query (or several).
pub fn scan_clauses(query: &str) -> BTreeSet<Clause> {
    let words = keyword_tokens(query);
    let mut out = BTreeSet::new();
    for (i, w) in words.iter().enumerate() {
        let next = words.get(i + 1).map(String::as_str);
        let prev = i.checked_sub(1).map(|j| words[j].as_str());
        match w.as_str() {
            "create" if next == Some("index") => {
                out.insert(Clause::IndexCreate);
            }
            "create" => {
                out.insert(Clause::Create);
            }
            "drop" if next == Some("index") => {
                out.insert(Clause::IndexDrop);
            }
            "match" => {
                out.insert(Clause::Match);
            }
            "merge" => {
                out.insert(Clause::Merge);
            }
            "detach" if next == Some("delete") => {
                out.insert(Clause::DetachDelete);
            }
            "delete" if prev != Some("detach") => {
                out.insert(Clause::Delete);
            }
            "remove" => {
                out.insert(Clause::Remove);
            }
            "storage" if next == Some("mode") => {
                out.insert(Clause::StorageMode);
            }
            _ => {}
        }
    }
    out
}

/// Lowercased `Word` tokens from the Core lexer. Strings and comments lex as
/// their own kinds, so a keyword inside one is never returned here.
fn keyword_tokens(query: &str) -> Vec<String> {
    lex(query)
        .iter()
        .filter(|token| token.kind == TokenKind::Word)
        .map(|token| token.text(query).to_ascii_lowercase())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::Clause::*;
    use super::*;

    fn scan(q: &str) -> Vec<Clause> {
        scan_clauses(q).into_iter().collect()
    }

    #[test]
    fn detects_single_clauses() {
        assert_eq!(scan("CREATE (n:Person) RETURN n"), vec![Create]);
        assert_eq!(scan("MERGE (n:X {id: 1})"), vec![Merge]);
        assert_eq!(scan("MATCH (n) REMOVE n.prop"), vec![Match, Remove]);
    }

    #[test]
    fn detects_multi_word_clauses() {
        assert_eq!(scan("CREATE INDEX ON :Person(name)"), vec![IndexCreate]);
        assert_eq!(scan("DROP INDEX ON :Person(name)"), vec![IndexDrop]);
        assert_eq!(scan("MATCH (n) DETACH DELETE n"), vec![Match, DetachDelete]);
        assert_eq!(scan("STORAGE MODE IN_MEMORY_ANALYTICAL"), vec![StorageMode]);
    }

    #[test]
    fn plain_delete_distinct_from_detach_delete() {
        assert_eq!(scan("MATCH (n) DELETE n"), vec![Match, Delete]);
        // DETACH DELETE yields only DetachDelete, not Delete.
        assert_eq!(scan("MATCH (n) DETACH DELETE n"), vec![Match, DetachDelete]);
    }

    #[test]
    fn create_index_is_not_plain_create() {
        let cs = scan_clauses("CREATE INDEX ON :Person(name)");
        assert!(cs.contains(&IndexCreate));
        assert!(!cs.contains(&Create));
    }

    #[test]
    fn case_insensitive() {
        assert_eq!(scan("create (n)"), vec![Create]);
        assert_eq!(scan("CrEaTe (n)"), vec![Create]);
    }

    #[test]
    fn keywords_inside_strings_and_comments_are_ignored() {
        assert_eq!(scan("CREATE (n {note: 'please match this'})"), vec![Create]);
        assert_eq!(scan("CREATE (n) // remember to match\n"), vec![Create]);
        assert_eq!(scan("CREATE (n) /* merge later */ RETURN n"), vec![Create]);
    }

    #[test]
    fn word_boundaries_prevent_false_matches() {
        // `created` is a property name, not CREATE.
        assert_eq!(scan("MATCH (n {created: true}) RETURN n"), vec![Match]);
    }

    #[test]
    fn composes_across_lines_of_one_query() {
        assert_eq!(scan("MATCH (a)\nCREATE (a)-[:R]->()"), vec![Create, Match]);
    }
}
