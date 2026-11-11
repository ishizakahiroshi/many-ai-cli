//! Every struct that owns a temporary folder declares it last.
//!
//! Fields drop in declaration order. A fixture that declared its `TempDir`
//! first removed the folder while a later field (a database connection, a
//! store, a service) still held a file inside it. Windows refuses that, and
//! `tempfile::TempDir` ignores the failure, so every test run left its folders
//! behind: 281 of them per run of this library's tests, 6,921 in four days,
//! with every test passing. This check reads the sources and fails as soon as
//! such a struct is written again.
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

struct Declared {
    line: usize,
    name: String,
    /// `(field, type)` in declaration order. `None` when a field spans lines
    /// and its type cannot be read by this line-based reader.
    fields: Option<Vec<(String, String)>>,
}

fn declared(source: &str) -> Vec<Declared> {
    let lines: Vec<&str> = source.lines().collect();
    let mut found = vec![];
    let mut at = 0;
    while at < lines.len() {
        let line = lines[at].trim();
        at += 1;
        let Some(rest) = line
            .split_once("struct ")
            .filter(|(before, _)| before.is_empty() || before.starts_with("pub"))
            .map(|(_, rest)| rest)
        else {
            continue;
        };
        if !line.ends_with('{') {
            continue;
        }
        let name: String = rest
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        let start = at;
        let mut fields = Some(vec![]);
        while at < lines.len() && lines[at].trim() != "}" {
            let field = lines[at].trim();
            at += 1;
            if field.is_empty() || field.starts_with("//") || field.starts_with("#[") {
                continue;
            }
            match field.strip_suffix(',').and_then(|f| f.split_once(':')) {
                Some((name, ty)) => {
                    let name = name.split_whitespace().last().unwrap_or_default();
                    if let Some(fields) = &mut fields {
                        fields.push((name.to_owned(), ty.trim().to_owned()));
                    }
                }
                None => fields = None,
            }
        }
        found.push(Declared {
            line: start,
            name,
            fields,
        });
    }
    found
}

/// The type itself, without its path and its generic arguments.
fn plain(ty: &str) -> &str {
    let ty = ty.split('<').next().unwrap_or(ty);
    ty.rsplit("::").next().unwrap_or(ty).trim()
}

fn owns(ty: &str, owners: &BTreeSet<String>) -> bool {
    ty.contains("TempDir") || owners.contains(plain(ty))
}

/// One line per struct whose temporary folder is not dropped last.
fn misplaced(sources: &[(PathBuf, String)]) -> Vec<String> {
    let all: Vec<(&Path, Declared)> = sources
        .iter()
        .flat_map(|(path, text)| declared(text).into_iter().map(move |d| (path.as_path(), d)))
        .collect();
    // A struct that holds such a struct owns the folder too.
    let mut owners = BTreeSet::new();
    loop {
        let before = owners.len();
        for (_, declared) in &all {
            if let Some(fields) = &declared.fields
                && fields.iter().any(|(_, ty)| owns(ty, &owners))
            {
                owners.insert(declared.name.clone());
            }
        }
        if owners.len() == before {
            break;
        }
    }
    let mut found = vec![];
    for (path, declared) in &all {
        let Some(fields) = &declared.fields else {
            continue;
        };
        let Some(first) = fields.iter().position(|(_, ty)| owns(ty, &owners)) else {
            continue;
        };
        if let Some((later, _)) = fields[first..].iter().find(|(_, ty)| !owns(ty, &owners)) {
            found.push(format!(
                "{}:{}: struct {}: `{}` owns a temporary folder but is declared before `{later}`",
                path.display(),
                declared.line,
                declared.name,
                fields[first].0,
            ));
        }
    }
    found
}

fn sources(dir: &Path, into: &mut Vec<(PathBuf, String)>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            sources(&path, into);
        } else if path.extension().is_some_and(|e| e == "rs") {
            let text = std::fs::read_to_string(&path).unwrap();
            into.push((path, text));
        }
    }
}

#[test]
fn every_struct_drops_its_temporary_folder_last() {
    let mut all = vec![];
    sources(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut all);
    assert!(all.len() > 100, "the sources were not found");
    let found = misplaced(&all);
    assert!(
        found.is_empty(),
        "Move the temporary folder to the end of the struct. Fields drop in declaration \
         order, and on Windows a folder cannot be removed while a later field still holds a \
         file inside it; the folder is then left behind in the temp directory without any \
         test failing.\n{}",
        found.join("\n")
    );
}

#[test]
fn the_check_reports_a_folder_that_is_not_declared_last() {
    let check = |text: &str| misplaced(&[(PathBuf::from("x.rs"), text.to_owned())]);
    let first = "struct Fixture {\n    dir: tempfile::TempDir,\n    db: Connection,\n}\n";
    assert_eq!(
        check(first),
        ["x.rs:1: struct Fixture: `dir` owns a temporary folder but is declared before `db`"]
    );
    let last = "struct Fixture {\n    db: Connection,\n    // why\n    dir: TempDir,\n}\n";
    assert!(check(last).is_empty());
    let two =
        "pub(crate) struct Fixture {\n    db: Db,\n    a: TempDir,\n    b: Option<TempDir>,\n}\n";
    assert!(check(two).is_empty());
    // A struct holding such a fixture owns the folder as well.
    let nested =
        format!("{last}struct Remote {{\n    fixture: Fixture,\n    service: Service,\n}}\n");
    assert_eq!(
        check(&nested),
        [
            "x.rs:6: struct Remote: `fixture` owns a temporary folder but is declared before \
             `service`"
        ]
    );
    let none = "struct Plain {\n    a: String,\n    b: PathBuf,\n}\n";
    assert!(check(none).is_empty());
}
