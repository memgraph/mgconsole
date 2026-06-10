//! Clause scanner (slice 27): a pure function from query text to the clauses
//! that matter for import ordering. Used by parser mode (28) and to enforce
//! vertices-first ordering in parallel import (31).
//!
//! It tokenises into lowercased identifier words, skipping string literals
//! (`'…'`, `"…"`, `` `…` ``) and comments (`// …`, `/* … */`), so a keyword
//! inside a string or comment is never detected, and word boundaries prevent
//! `created` from matching `create`. No database.

use std::collections::BTreeSet;

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

/// Lowercased identifier words, skipping string literals and comments.
fn keyword_tokens(query: &str) -> Vec<String> {
    let chars: Vec<char> = query.chars().collect();
    let mut words = Vec::new();
    let mut cur = String::new();
    let mut i = 0;

    let flush = |cur: &mut String, words: &mut Vec<String>| {
        if !cur.is_empty() {
            words.push(std::mem::take(cur));
        }
    };

    while i < chars.len() {
        let c = chars[i];
        match c {
            '\'' | '"' | '`' => {
                flush(&mut cur, &mut words);
                i = skip_string(&chars, i, c);
                continue;
            }
            '/' if chars.get(i + 1) == Some(&'/') => {
                flush(&mut cur, &mut words);
                i = skip_to(&chars, i + 2, '\n');
                continue;
            }
            '/' if chars.get(i + 1) == Some(&'*') => {
                flush(&mut cur, &mut words);
                i = skip_block_comment(&chars, i + 2);
                continue;
            }
            _ if c.is_alphanumeric() || c == '_' => cur.push(c.to_ascii_lowercase()),
            _ => flush(&mut cur, &mut words),
        }
        i += 1;
    }
    flush(&mut cur, &mut words);
    words
}

/// Index just past the closing `quote` (honours `\` escapes; doubled backticks).
fn skip_string(chars: &[char], open: usize, quote: char) -> usize {
    let mut i = open + 1;
    while i < chars.len() {
        let c = chars[i];
        if quote != '`' && c == '\\' {
            i += 2;
            continue;
        }
        if c == quote {
            if quote == '`' && chars.get(i + 1) == Some(&'`') {
                i += 2;
                continue;
            }
            return i + 1;
        }
        i += 1;
    }
    i
}

fn skip_to(chars: &[char], from: usize, target: char) -> usize {
    let mut i = from;
    while i < chars.len() && chars[i] != target {
        i += 1;
    }
    i
}

fn skip_block_comment(chars: &[char], from: usize) -> usize {
    let mut i = from;
    while i < chars.len() {
        if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
            return i + 2;
        }
        i += 1;
    }
    i
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
