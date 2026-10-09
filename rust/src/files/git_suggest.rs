//! Deterministic commit-message drafting from the frozen Go diff heuristics.
use super::{StatusFile, sanitize_message};
use std::collections::BTreeMap;
#[derive(Default)]
struct Analysis {
    added: Vec<String>,
    deleted: Vec<String>,
    modified: Vec<String>,
    renamed: Vec<String>,
    deps: Vec<String>,
    routes: Vec<String>,
    removed_routes: Vec<String>,
    funcs: Vec<String>,
    deleted_funcs: Vec<String>,
    types: Vec<String>,
    renames: Vec<String>,
    func_sites: BTreeMap<String, String>,
    deleted_sites: BTreeMap<String, String>,
    type_sites: BTreeMap<String, String>,
    route_sites: BTreeMap<String, String>,
    i18n: i64,
    loc_added: i64,
    loc_deleted: i64,
    handling: i64,
    prefix: String,
    scope: String,
    dominant: String,
    dominant_status: String,
    deps_only: bool,
}
fn base(p: &str) -> String {
    p.replace('\\', "/").rsplit('/').next().unwrap_or("").into()
}
fn unique(items: &mut Vec<String>, item: &str) {
    if !items.iter().any(|s| s == item) {
        items.push(item.into());
    }
}
fn capture(pattern: &str, line: &str) -> Option<String> {
    regex::Regex::new(pattern)
        .ok()?
        .captures(line)?
        .get(1)
        .map(|m| m.as_str().to_owned())
}
fn route(line: &str) -> Option<String> {
    for p in [
        r#"mux\.HandleFunc\("([^"]+)""#,
        r#"\b(?:app|router)\.(?:get|post|put|delete|patch)\(\s*['"]([^'"]+)"#,
        r#"^[+-]@(?:app|router|bp)\.(?:get|post|put|delete|patch|route)\(\s*['"]([^'"]+)"#,
    ] {
        if let Some(c) = capture(p, line) {
            return Some(c);
        }
    }
    None
}
fn matches(p: &str, line: &str) -> bool {
    regex::Regex::new(p).is_ok_and(|r| r.is_match(line))
}
fn modified(s: &str) -> bool {
    !matches!(s, "" | "A" | "??" | "D" | "R")
}
fn pick(groups: &[(&Vec<String>, bool)]) -> (String, usize) {
    for (g, b) in groups {
        if let Some(first) = g.first() {
            return (if *b { base(first) } else { first.clone() }, g.len());
        }
    }
    (String::new(), 0)
}
fn more(head: &str, count: usize, ja: bool) -> String {
    if count <= 1 {
        head.into()
    } else if ja {
        format!("{head} ほか {} 件", count - 1)
    } else {
        format!("{head} (+{} more)", count - 1)
    }
}
fn list(items: &[String], limit: usize, ja: bool, basenames: bool) -> String {
    let v: Vec<_> = items
        .iter()
        .take(limit)
        .map(|s| if basenames { base(s) } else { s.clone() })
        .collect();
    let out = v.join(", ");
    if items.len() <= limit {
        out
    } else if ja {
        format!("{out} ほか {} 件", items.len() - limit)
    } else {
        format!("{out} (+{} more)", items.len() - limit)
    }
}
fn scope(paths: &[String]) -> String {
    let mut common: Vec<String> = vec![];
    for (i, p) in paths.iter().enumerate() {
        let mut parts: Vec<_> = p.split('/').map(str::to_owned).collect();
        parts.pop();
        if i == 0 {
            common = parts;
        } else {
            let n = common
                .iter()
                .zip(parts.iter())
                .take_while(|(a, b)| a == b)
                .count();
            common.truncate(n);
        }
    }
    common
        .into_iter()
        .rev()
        .find(|s| !matches!(s.as_str(), "src" | "internal" | "cmd" | "pkg" | "lib"))
        .unwrap_or_default()
}
impl Analysis {
    fn scan(&mut self, diff: &str) {
        let mut file = String::new();
        let mut old = String::new();
        for line in diff.lines() {
            if let Some(p) = line.strip_prefix("+++ b/") {
                file = p.into();
                continue;
            }
            if let Some(p) = line.strip_prefix("rename from ") {
                old = p.trim().into();
                continue;
            }
            if let Some(p) = line.strip_prefix("rename to ") {
                if !old.is_empty() {
                    self.renames
                        .push(format!("{} → {}", base(&old), base(p.trim())));
                    old.clear();
                }
                continue;
            }
            if line.starts_with('+') && !line.starts_with("++") {
                self.loc_added += 1;
                if matches(r"^\+\s*if\s+err\s*!=", line)
                    || matches(r"^\+\s*throw\s", line)
                    || matches(r"^\+\s*(?:\}\s*)?catch\b", line)
                {
                    self.handling += 1;
                }
                if let Some(r) = route(line) {
                    unique(&mut self.routes, &r);
                    self.route_sites.entry(r).or_insert(file.clone());
                }
                let mut funcs = Vec::new();
                let mut types = Vec::new();
                if file.ends_with(".go") {
                    funcs.push(r"^\+func (?:\([^)]*\)\s*)?([A-Za-z0-9_]+)\(");
                    types.push(r"^\+type ([A-Za-z0-9_]+) (?:struct|interface)\b");
                }
                if [".ts", ".tsx", ".js", ".jsx", ".mts", ".mjs", ".cjs"]
                    .iter()
                    .any(|e| file.to_lowercase().ends_with(e))
                {
                    funcs.extend([
                        r"^\+export (?:default )?(?:async )?function ([A-Za-z0-9_$]+)",
                        r"^\+export const ([A-Za-z0-9_$]+)\s*=",
                        r"^\+export (?:default )?(?:abstract )?class ([A-Za-z0-9_$]+)",
                    ]);
                    types.push(r"^\+export (?:type|interface) ([A-Za-z0-9_$]+)");
                }
                if file.ends_with(".py") {
                    funcs.push(r"^\+def ([A-Za-z0-9_]+)\(");
                    types.push(r"^\+class ([A-Za-z0-9_]+)");
                }
                for re in funcs {
                    if let Some(n) = capture(re, line) {
                        unique(&mut self.funcs, &n);
                        self.func_sites.entry(n).or_insert(file.clone());
                        break;
                    }
                }
                for re in types {
                    if let Some(n) = capture(re, line) {
                        unique(&mut self.types, &n);
                        self.type_sites.entry(n).or_insert(file.clone());
                    }
                }
                let lower = file.to_lowercase();
                if (lower.contains("i18n") || lower.contains("locales"))
                    && [".json", ".ts", ".js"].iter().any(|e| lower.ends_with(e))
                    && matches(r#"^\+\s*['"]?[A-Za-z0-9_.-]+['"]?\s*:"#, line)
                {
                    self.i18n += 1;
                }
            } else if line.starts_with('-') && !line.starts_with("--") {
                self.loc_deleted += 1;
                if let Some(r) = route(line) {
                    unique(&mut self.removed_routes, &r);
                }
                if file.ends_with(".go")
                    && let Some(n) = capture(r"^-func (?:\([^)]*\)\s*)?([A-Za-z0-9_]+)\(", line)
                {
                    unique(&mut self.deleted_funcs, &n);
                    self.deleted_sites.entry(n).or_insert(file.clone());
                }
            }
        }
        let common: Vec<_> = self
            .routes
            .iter()
            .filter(|r| self.removed_routes.contains(r))
            .cloned()
            .collect();
        self.routes.retain(|r| !common.contains(r));
        self.removed_routes.retain(|r| !common.contains(r));
    }
    fn added_symbol(&self) -> (String, usize) {
        pick(&[
            (&self.routes, false),
            (&self.types, false),
            (&self.funcs, false),
            (&self.added, true),
        ])
    }
    fn changed_symbol(&self) -> (String, usize) {
        if !self.funcs.is_empty() || !self.types.is_empty() {
            return pick(&[(&self.funcs, false), (&self.types, false)]);
        }
        if !self.modified.is_empty() && modified(&self.dominant_status) {
            return (base(&self.dominant), self.modified.len());
        }
        pick(&[
            (&self.modified, true),
            (&self.added, true),
            (&self.deleted, true),
            (&self.renames, false),
        ])
    }
    fn verb_symbol(&self) -> (&str, String, usize) {
        let additions = !self.added.is_empty()
            || !self.funcs.is_empty()
            || !self.types.is_empty()
            || !self.routes.is_empty();
        let removals = !self.deleted.is_empty() || !self.removed_routes.is_empty();
        let moved: Vec<_> = self
            .funcs
            .iter()
            .filter(|f| {
                self.deleted_sites
                    .get(*f)
                    .is_some_and(|site| self.func_sites.get(*f) != Some(site))
            })
            .cloned()
            .collect();
        let mut dominant = (String::new(), 0);
        for (items, sites) in [
            (&self.routes, &self.route_sites),
            (&self.types, &self.type_sites),
            (&self.funcs, &self.func_sites),
        ] {
            let group: Vec<_> = items
                .iter()
                .filter(|i| sites.get(*i) == Some(&self.dominant))
                .cloned()
                .collect();
            if !group.is_empty() {
                dominant = (group[0].clone(), group.len());
                break;
            }
        }
        let (verb, (head, count)) = if self.deps_only {
            ("bump", pick(&[(&self.deps, true)]))
        } else if !self.renames.is_empty() && !additions && !removals && self.modified.is_empty() {
            ("rename", pick(&[(&self.renames, false)]))
        } else if !moved.is_empty() {
            ("move", (moved[0].clone(), moved.len()))
        } else if modified(&self.dominant_status) && !self.added.is_empty() && dominant.0.is_empty()
        {
            ("change", (base(&self.dominant), self.modified.len()))
        } else if modified(&self.dominant_status)
            && !self.added.is_empty()
            && !dominant.0.is_empty()
            && !removals
        {
            ("add", dominant)
        } else if additions && !removals {
            ("add", self.added_symbol())
        } else if removals && !additions {
            (
                "remove",
                pick(&[
                    (&self.removed_routes, false),
                    (&self.deleted, true),
                    (&self.deleted_funcs, false),
                ]),
            )
        } else if self.handling >= 3 && self.handling * 2 >= self.loc_added {
            ("handle", self.changed_symbol())
        } else if self.loc_deleted >= 15 && self.loc_added * 3 < self.loc_deleted * 2 {
            ("simplify", self.changed_symbol())
        } else {
            (
                if matches!(self.prefix.as_str(), "docs" | "style" | "chore" | "test") {
                    "update"
                } else {
                    "refactor"
                },
                self.changed_symbol(),
            )
        };
        (verb, head, count)
    }
}
pub(super) fn suggest(
    files: &[StatusFile],
    stat: &str,
    diff: &str,
    notice: &str,
    language: &str,
    weights: &BTreeMap<String, i64>,
) -> (String, String) {
    let ja = language.is_empty() || language.eq_ignore_ascii_case("ja");
    let mut a = Analysis::default();
    let (mut docs, mut tests, mut deps, mut style, mut code) = (true, true, true, true, false);
    let mut paths = Vec::new();
    let mut best = (-1i64, -1i64, String::new());
    for f in files {
        let p = f.path.replace('\\', "/");
        if p.is_empty() {
            continue;
        }
        paths.push(p.clone());
        match f.status.as_str() {
            "A" | "??" => a.added.push(p.clone()),
            "D" => a.deleted.push(p.clone()),
            "R" => a.renamed.push(p.clone()),
            _ => a.modified.push(p.clone()),
        };
        if matches!(
            base(&p).as_str(),
            "go.mod"
                | "go.sum"
                | "go.work"
                | "go.work.sum"
                | "package.json"
                | "package-lock.json"
                | "bun.lockb"
                | "yarn.lock"
                | "pnpm-lock.yaml"
        ) {
            a.deps.push(p.clone());
        } else {
            deps = false;
        }
        docs &= p.starts_with("docs/")
            || p.starts_with("README")
            || p.starts_with("CHANGELOG")
            || p.ends_with(".md");
        tests &= p.ends_with("_test.go") || p.contains(".test.") || p.contains(".spec.");
        style &= p.ends_with(".css") || p.ends_with(".scss");
        code |= ["web/", "internal/", "cmd/", "pkg/"]
            .iter()
            .any(|s| p.starts_with(s));
        let w = *weights.get(&p).unwrap_or(&0);
        let rank = match f.status.as_str() {
            "A" | "??" => 3,
            "D" => 1,
            _ => 2,
        };
        if w > best.0
            || (w == best.0
                && (rank > best.1 || (rank == best.1 && (best.2.is_empty() || p < best.2))))
        {
            best = (w, rank, p.clone());
            a.dominant = p;
            a.dominant_status = f.status.clone();
        }
    }
    a.deps_only = !paths.is_empty() && deps;
    a.scan(diff);
    a.scope = scope(&paths);
    let adds =
        !a.added.is_empty() || !a.funcs.is_empty() || !a.types.is_empty() || !a.routes.is_empty();
    let removals = !a.deleted.is_empty() || !a.removed_routes.is_empty();
    a.prefix = if paths.is_empty() {
        "chore"
    } else if a.deps_only {
        "chore(deps)"
    } else if docs {
        "docs"
    } else if tests {
        "test"
    } else if style {
        "style"
    } else if !a.renamed.is_empty()
        && a.added.is_empty()
        && a.deleted.is_empty()
        && a.modified.is_empty()
    {
        "refactor"
    } else if adds {
        "feat"
    } else if code && (removals || !a.modified.is_empty()) {
        "refactor"
    } else if code {
        "feat"
    } else {
        "chore"
    }
    .into();
    let (verb, head, count) = a.verb_symbol();
    let prefix = if a.scope.is_empty() || a.scope == a.prefix || a.prefix.contains('(') {
        a.prefix.clone()
    } else {
        format!("{}({})", a.prefix, a.scope)
    };
    let mut subject = if head.is_empty() {
        format!("{prefix}: {}", if ja { "変更なし" } else { "no changes" })
    } else {
        let h = more(&head, count, ja);
        let phrase = if ja {
            match verb {
                "handle" => format!("{h} にエラー処理を追加"),
                "rename" => format!("{h} に改名"),
                _ => format!(
                    "{h} を{}",
                    match verb {
                        "add" => "追加",
                        "remove" => "削除",
                        "move" => "移動",
                        "bump" | "update" => "更新",
                        "simplify" => "簡潔化",
                        "test" => "テストを追加",
                        "change" => "変更",
                        _ => "整理",
                    }
                ),
            }
        } else {
            format!(
                "{} {h}",
                match verb {
                    "refactor" => "rework",
                    "handle" => "handle errors in",
                    "test" => "add tests for",
                    v => v,
                }
            )
        };
        format!("{prefix}: {phrase}")
    };
    if verb == "change" && !a.added.is_empty() {
        let new = more(&base(&a.added[0]), a.added.len(), ja);
        subject.push_str(&if ja {
            format!("（{new} 新規）")
        } else {
            format!(" (new: {new})")
        });
    }
    let mut count = if ja {
        format!(
            "ファイル {} 件（新規 {} / 変更 {} / 削除 {}",
            files.len(),
            a.added.len(),
            a.modified.len(),
            a.deleted.len()
        )
    } else {
        format!(
            "{} file(s): {} added / {} modified / {} deleted",
            files.len(),
            a.added.len(),
            a.modified.len(),
            a.deleted.len()
        )
    };
    if !a.renamed.is_empty() {
        count.push_str(&if ja {
            format!(" / 改名 {}", a.renamed.len())
        } else {
            format!(" / {} renamed", a.renamed.len())
        });
    }
    count.push_str(if ja { "）。" } else { "." });
    let mut body = vec![count];
    for (items, jp, en, basenames, limit, suffix) in [
        (&a.added, "新規", "New", true, 5, ""),
        (&a.deleted, "削除", "Removed", true, 5, ""),
        (&a.renames, "改名", "Renamed", false, 5, ""),
        (&a.deps, "依存", "Deps", true, 5, "dep"),
        (&a.routes, "API追加", "API added", false, 6, ""),
        (&a.removed_routes, "API削除", "API removed", false, 6, ""),
    ] {
        if !items.is_empty() {
            let value = list(items, limit, ja, basenames);
            body.push(if suffix == "dep" {
                if ja {
                    format!("- {jp}: {value} を更新")
                } else {
                    format!("- {en}: updated {value}")
                }
            } else {
                format!("- {}: {value}", if ja { jp } else { en })
            });
        }
    }
    if a.i18n > 0 {
        body.push(if ja {
            format!("- i18n: {} 件のキーを追加", a.i18n)
        } else {
            format!("- i18n: added {} key(s)", a.i18n)
        });
    }
    for (items, jp, en) in [(&a.types, "型", "Types"), (&a.funcs, "関数", "Functions")] {
        if items.is_empty() || (en == "Functions" && !a.routes.is_empty()) {
            continue;
        }
        let v = list(items, 6, ja, false);
        body.push(if ja {
            format!("- {jp}: {v} を追加")
        } else {
            format!("- {en}: added {v}")
        });
    }
    let mut body = body.join("\n");
    if !stat.is_empty() {
        body.push_str(&format!("\n\n{stat}"));
    }
    if !notice.is_empty() {
        body.push_str(&format!("\n\n{notice}"));
    }
    (
        sanitize_message(&subject, 200),
        sanitize_message(&body, 8192),
    )
}
