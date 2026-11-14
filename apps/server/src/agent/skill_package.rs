//! Agent Skills packages (<https://agentskills.io/specification>): a folder with `SKILL.md` (YAML
//! front matter with `name` and `description`, then a Markdown body) and optional files such as
//! `scripts/`, `references/` and `assets/`. This module reads one from a `.zip` or a public GitHub
//! folder (also a skills.sh entry, which names a GitHub repository) and checks it. A package is
//! untrusted input: paths stay below the skill folder, symbolic links are refused, and every size
//! is limited before and while it is read. Nothing here runs a script.

use std::io::{Cursor, Read};

use serde::Deserialize;
use url::Url;

use super::outbound::{Outbound, OutboundError};

pub const MAX_NAME_CHARS: usize = 64;
pub const MAX_DESCRIPTION_CHARS: usize = 1_024;
/// `SKILL.md` (front matter and body).
pub const MAX_SKILL_MD_BYTES: usize = 256 * 1024;
/// Files besides `SKILL.md`.
pub const MAX_FILES: usize = 100;
pub const MAX_FILE_BYTES: usize = 512 * 1024;
/// `SKILL.md` and every file together.
pub const MAX_TOTAL_BYTES: usize = 2 * 1024 * 1024;
/// An uploaded `.zip`.
pub const MAX_ZIP_BYTES: usize = 4 * 1024 * 1024;
/// Entries of a `.zip`, folders and skipped files included.
const MAX_ZIP_ENTRIES: usize = 500;
/// GitHub's listing of a repository.
const MAX_TREE_BYTES: usize = 8 * 1024 * 1024;
const MAX_PATH_CHARS: usize = 200;
const MAX_PATH_DEPTH: usize = 8;

/// One file of a skill, by its path below the skill folder.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SkillFile {
    pub path: String,
    pub content: Vec<u8>,
}

/// A checked skill.
#[derive(Clone, Debug)]
pub struct SkillPackage {
    pub name: String,
    pub description: String,
    pub body: String,
    pub files: Vec<SkillFile>,
}

/// A name as the spec wants it: 1–64 lowercase letters, digits and hyphens, no hyphen at either
/// end and no two in a row.
pub fn checked_name(raw: &str) -> Result<String, String> {
    let name = raw.trim();
    let valid = !name.is_empty()
        && name.chars().count() <= MAX_NAME_CHARS
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !name.starts_with('-')
        && !name.ends_with('-')
        && !name.contains("--");
    if valid {
        Ok(name.to_owned())
    } else {
        Err(format!(
            "The name \"{}\" is not valid: use 1–64 lowercase letters, digits and single hyphens (for example pdf-forms).",
            name.chars().take(80).collect::<String>()
        ))
    }
}

/// The description on one line: 1–1,024 characters, no control characters.
pub fn checked_description(raw: &str) -> Result<String, String> {
    let description = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if description.is_empty() {
        return Err("The skill needs a description: what it does and when to use it.".to_owned());
    }
    if description.chars().count() > MAX_DESCRIPTION_CHARS {
        return Err("The description may have at most 1,024 characters.".to_owned());
    }
    if description.chars().any(char::is_control) {
        return Err("The description may not contain control characters.".to_owned());
    }
    Ok(description)
}

/// The Markdown body: `\n` line breaks, no NUL, at most 256 KiB.
pub fn checked_body(raw: &str) -> Result<String, String> {
    let body = raw.replace("\r\n", "\n");
    let body = body.trim_matches('\n');
    if body.len() > MAX_SKILL_MD_BYTES {
        return Err("SKILL.md may have at most 256 KiB.".to_owned());
    }
    if body.contains('\0') {
        return Err("SKILL.md may not contain NUL characters.".to_owned());
    }
    Ok(body.to_owned())
}

/// A file path below the skill folder, or `None` when it could leave it: relative, `/` only, no
/// empty, `.` or `..` part, no `\`, `:` or control characters, at most 8 levels.
#[must_use]
pub fn safe_path(raw: &str) -> Option<String> {
    if raw.is_empty()
        || raw.chars().count() > MAX_PATH_CHARS
        || raw.starts_with('/')
        || raw.chars().any(|c| c.is_control() || c == '\\' || c == ':')
    {
        return None;
    }
    let parts: Vec<&str> = raw.split('/').collect();
    if parts.len() > MAX_PATH_DEPTH
        || parts
            .iter()
            .any(|part| part.is_empty() || *part == "." || *part == "..")
    {
        return None;
    }
    Some(raw.to_owned())
}

/// `name`, `description` and the body of a `SKILL.md`.
pub fn parse_skill_md(text: &str) -> Result<(String, String, String), String> {
    let text = text.trim_start_matches('\u{feff}').replace("\r\n", "\n");
    let rest = text
        .strip_prefix("---\n")
        .ok_or("SKILL.md must start with YAML front matter between --- lines.")?;
    let mut front = Vec::new();
    let mut body = None;
    let mut lines = rest.split_inclusive('\n');
    for line in lines.by_ref() {
        let bare = line.trim_end();
        if bare == "---" || bare == "..." {
            body = Some(lines.collect::<String>());
            break;
        }
        front.push(bare);
    }
    let body = body.ok_or("The front matter of SKILL.md has no closing --- line.")?;
    let fields = front_matter(&front);
    let field = |key: &str| {
        fields
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
    };
    let name = checked_name(field("name").ok_or("The front matter has no name.")?)?;
    let description = checked_description(field("description").unwrap_or_default())?;
    Ok((name, description, checked_body(&body)?))
}

/// The top-level scalar fields of a small YAML mapping: plain, quoted, folded (`>`) and literal
/// (`|`) values, and plain values that go on over indented lines. Nested mappings and lists
/// (`metadata:`, `allowed-tools:` as a list) are skipped: only `name` and `description` are used.
fn front_matter(lines: &[&str]) -> Vec<(String, String)> {
    let mut fields = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        index += 1;
        if line.starts_with([' ', '\t', '#', '-']) || line.trim().is_empty() {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim().to_owned();
        let value = value.trim();
        // The indented lines that belong to this key.
        let start = index;
        while index < lines.len()
            && (lines[index].starts_with([' ', '\t']) || lines[index].trim().is_empty())
        {
            index += 1;
        }
        let more: Vec<&str> = lines[start..index].to_vec();
        let parsed = if value.starts_with('|') || value.starts_with('>') {
            let indent = more
                .iter()
                .filter(|line| !line.trim().is_empty())
                .map(|line| line.len() - line.trim_start().len())
                .min()
                .unwrap_or(0);
            let block: Vec<&str> = more
                .iter()
                .map(|line| line.get(indent..).unwrap_or("").trim_end())
                .collect();
            if value.starts_with('|') {
                block.join("\n").trim().to_owned()
            } else {
                block.join(" ").trim().to_owned()
            }
        } else if let Some(quoted) = value.strip_prefix('"') {
            let joined = std::iter::once(quoted)
                .chain(more.iter().map(|line| line.trim()))
                .collect::<Vec<_>>()
                .join(" ");
            double_quoted(&joined)
        } else if let Some(quoted) = value.strip_prefix('\'') {
            let joined = std::iter::once(quoted)
                .chain(more.iter().map(|line| line.trim()))
                .collect::<Vec<_>>()
                .join(" ");
            let end = joined.rfind('\'').unwrap_or(joined.len());
            joined[..end].replace("''", "'")
        } else if value.is_empty() {
            // A nested mapping or list.
            continue;
        } else {
            std::iter::once(value)
                .chain(more.iter().map(|line| line.trim()))
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join(" ")
        };
        fields.push((key, parsed));
    }
    fields
}

/// The text of a YAML double-quoted value (after its opening quote).
fn double_quoted(raw: &str) -> String {
    let mut text = String::new();
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => break,
            '\\' => match chars.next() {
                Some('n') => text.push('\n'),
                Some('t') => text.push('\t'),
                Some(other) => text.push(other),
                None => break,
            },
            other => text.push(other),
        }
    }
    text
}

/// A package from `SKILL.md` and the other files, with every limit checked.
pub fn package(skill_md: &[u8], files: Vec<SkillFile>) -> Result<SkillPackage, String> {
    if skill_md.len() > MAX_SKILL_MD_BYTES {
        return Err("SKILL.md may have at most 256 KiB.".to_owned());
    }
    let text = std::str::from_utf8(skill_md).map_err(|_| "SKILL.md is not UTF-8 text.")?;
    let (name, description, body) = parse_skill_md(text)?;
    if files.len() > MAX_FILES {
        return Err(format!(
            "A skill may have at most {MAX_FILES} files besides SKILL.md."
        ));
    }
    let mut total = skill_md.len();
    for file in &files {
        if safe_path(&file.path).is_none() {
            return Err(format!("The path {} is not allowed.", file.path));
        }
        if file.content.len() > MAX_FILE_BYTES {
            return Err(format!("{} is larger than 512 KiB.", file.path));
        }
        total += file.content.len();
    }
    if total > MAX_TOTAL_BYTES {
        return Err("A skill may have at most 2 MiB of files.".to_owned());
    }
    Ok(SkillPackage {
        name,
        description,
        body,
        files,
    })
}

/// Files that zip tools add and a skill never needs.
fn skipped(path: &str) -> bool {
    path.starts_with("__MACOSX/") || path.rsplit('/').next() == Some(".DS_Store")
}

/// A package from an uploaded `.zip`: `SKILL.md` at its top, or in its one top folder. Any entry
/// whose path could leave the folder, any symbolic link and any encrypted entry refuse the whole
/// file; sizes are checked while reading, not only from the zip's own (untrusted) numbers.
pub fn from_zip(bytes: &[u8]) -> Result<SkillPackage, String> {
    if bytes.len() > MAX_ZIP_BYTES {
        return Err("The .zip may have at most 4 MiB.".to_owned());
    }
    let mut archive =
        zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| "The file is not a valid .zip.")?;
    if archive.len() > MAX_ZIP_ENTRIES {
        return Err(format!(
            "The .zip may have at most {MAX_ZIP_ENTRIES} entries."
        ));
    }
    let mut entries = Vec::new();
    for index in 0..archive.len() {
        let entry = archive
            .by_index_raw(index)
            .map_err(|_| "The .zip could not be read.")?;
        let name = entry
            .name()
            .map_err(|_| "The .zip has an entry name that is not valid.")?
            .into_owned();
        if entry.is_symlink() {
            return Err(format!(
                "The .zip has a symbolic link ({name}); skills may not have links."
            ));
        }
        if entry.encrypted() {
            return Err("The .zip has encrypted entries.".to_owned());
        }
        let is_dir = entry.is_dir();
        let path = name.trim_end_matches('/');
        if path.is_empty() && is_dir {
            continue;
        }
        if safe_path(path).is_none() {
            return Err(format!(
                "The .zip has an entry whose path is not allowed ({}).",
                name.chars().take(120).collect::<String>()
            ));
        }
        if !is_dir && !skipped(path) {
            entries.push((index, path.to_owned(), entry.size()));
        }
    }
    let prefix = if entries.iter().any(|(_, path, _)| path == "SKILL.md") {
        String::new()
    } else {
        let tops: Vec<&str> = entries
            .iter()
            .filter_map(|(_, path, _)| path.strip_suffix("/SKILL.md"))
            .filter(|folder| !folder.contains('/'))
            .collect();
        match tops.as_slice() {
            [folder] => format!("{folder}/"),
            [] => {
                return Err("The .zip has no SKILL.md at its top or in one top folder.".to_owned());
            }
            _ => {
                return Err(
                    "The .zip has more than one skill; upload one skill per .zip.".to_owned(),
                );
            }
        }
    };
    let mut skill_md = None;
    let mut files = Vec::new();
    let mut total = 0usize;
    for (index, path, declared) in entries {
        let Some(relative) = path.strip_prefix(&prefix) else {
            continue;
        };
        let limit = if relative == "SKILL.md" {
            MAX_SKILL_MD_BYTES
        } else {
            MAX_FILE_BYTES
        };
        if declared > limit as u64 {
            return Err(format!("{relative} is too large."));
        }
        let mut content = Vec::new();
        archive
            .by_index(index)
            .map_err(|_| "The .zip could not be read.")?
            .take(limit as u64 + 1)
            .read_to_end(&mut content)
            .map_err(|_| "The .zip could not be read.")?;
        if content.len() > limit {
            return Err(format!("{relative} is too large."));
        }
        total += content.len();
        if total > MAX_TOTAL_BYTES {
            return Err("A skill may have at most 2 MiB of files.".to_owned());
        }
        if relative == "SKILL.md" {
            skill_md = Some(content);
        } else {
            if files.len() >= MAX_FILES {
                return Err(format!(
                    "A skill may have at most {MAX_FILES} files besides SKILL.md."
                ));
            }
            files.push(SkillFile {
                path: relative.to_owned(),
                content,
            });
        }
    }
    package(&skill_md.ok_or("The .zip has no SKILL.md.")?, files)
}

// ---------------------------------------------------------------------------------------------
// GitHub and skills.sh

/// Where a skill is on GitHub.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitSource {
    /// `github` or `skills_sh`.
    pub kind: &'static str,
    pub owner: String,
    pub repo: String,
    /// A branch, tag or commit; `HEAD`: the default branch.
    pub git_ref: String,
    /// The skill folder, or where to look for `skill`; `""`: the repository's top.
    pub path: String,
    /// Find the folder with this name below `path` (skills.sh, `owner/repo@skill`).
    pub skill: Option<String>,
    /// The address as given, for the skill's source.
    pub url: String,
}

/// The GitHub hosts Orbit reads from. Tests point them at a local server (which must also be a
/// test host of the integrations' guard).
#[derive(Clone, Debug)]
pub struct SkillSources {
    pub api: Url,
    pub raw: Url,
}

impl Default for SkillSources {
    fn default() -> Self {
        Self {
            api: Url::parse("https://api.github.com").expect("a valid URL"),
            raw: Url::parse("https://raw.githubusercontent.com").expect("a valid URL"),
        }
    }
}

impl SkillSources {
    /// `ORBIT_AGENT_SKILLS_GITHUB_API` and `ORBIT_AGENT_SKILLS_GITHUB_RAW`: for local tests only.
    #[must_use]
    pub fn from_env() -> Self {
        let mut sources = Self::default();
        let read = |key: &str| {
            std::env::var(key)
                .ok()
                .and_then(|value| Url::parse(value.trim()).ok())
        };
        if let Some(api) = read("ORBIT_AGENT_SKILLS_GITHUB_API") {
            tracing::warn!(%api, "ORBIT_AGENT_SKILLS_GITHUB_API is set: for local tests only");
            sources.api = api;
        }
        if let Some(raw) = read("ORBIT_AGENT_SKILLS_GITHUB_RAW") {
            sources.raw = raw;
        }
        sources
    }
}

fn owner_ok(owner: &str) -> bool {
    (1..=39).contains(&owner.len()) && owner.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

fn repo_ok(repo: &str) -> bool {
    (1..=100).contains(&repo.len())
        && repo != "."
        && repo != ".."
        && repo
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

fn ref_ok(git_ref: &str) -> bool {
    (1..=200).contains(&git_ref.len())
        && !git_ref.contains("..")
        && !git_ref.starts_with('/')
        && git_ref
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/'))
}

/// Reads a source an admin entered: a GitHub folder (`https://github.com/owner/repo/tree/ref/path`,
/// a repository, or a `SKILL.md` link), a skills.sh entry (`https://skills.sh/owner/repo/skill`),
/// or the skills CLI forms `owner/repo`, `owner/repo/path` and `owner/repo@skill`.
pub fn parse_source(raw: &str) -> Result<GitSource, String> {
    let raw = raw.trim();
    let refuse = || {
        "Enter a GitHub folder (https://github.com/owner/repo/tree/main/skills/name), a skills.sh page (https://skills.sh/owner/repo/name) or owner/repo@name.".to_owned()
    };
    let (kind, parts): (&'static str, Vec<String>) = if raw.contains("://") {
        let url = Url::parse(raw).map_err(|_| refuse())?;
        if url.scheme() != "https" || url.port().is_some() || !url.username().is_empty() {
            return Err(refuse());
        }
        let kind = match url.host_str() {
            Some("github.com" | "www.github.com") => "github",
            Some("skills.sh" | "www.skills.sh") => "skills_sh",
            _ => return Err("Skills install only from github.com or skills.sh.".to_owned()),
        };
        let parts = url
            .path_segments()
            .map(|segments| {
                segments
                    .filter(|part| !part.is_empty())
                    .map(|part| {
                        percent_encoding::percent_decode_str(part)
                            .decode_utf8_lossy()
                            .into_owned()
                    })
                    .collect()
            })
            .unwrap_or_default();
        (kind, parts)
    } else {
        let (path, skill) = match raw.split_once('@') {
            Some((path, skill)) => (path, Some(skill)),
            None => (raw, None),
        };
        let mut parts: Vec<String> = path
            .split('/')
            .filter(|part| !part.is_empty())
            .map(str::to_owned)
            .collect();
        if let Some(skill) = skill {
            if parts.len() != 2 {
                return Err(refuse());
            }
            parts.push(skill.to_owned());
            ("skills_sh", parts)
        } else {
            if parts.len() < 2 {
                return Err(refuse());
            }
            // `owner/repo/path`: a folder on the default branch.
            let mut github = vec![parts[0].clone(), parts[1].clone()];
            if parts.len() > 2 {
                github.push("tree".to_owned());
                github.push("HEAD".to_owned());
                github.extend(parts.drain(2..));
            }
            ("github", github)
        }
    };
    let [owner, repo, rest @ ..] = parts.as_slice() else {
        return Err(refuse());
    };
    let repo = repo.strip_suffix(".git").unwrap_or(repo);
    if !owner_ok(owner) || !repo_ok(repo) {
        return Err(refuse());
    }
    let (git_ref, path, skill) = if kind == "skills_sh" {
        let [skill] = rest else {
            return Err(refuse());
        };
        let skill = checked_name(skill)?;
        ("HEAD".to_owned(), String::new(), Some(skill))
    } else {
        match rest {
            [] => ("HEAD".to_owned(), String::new(), None),
            [mode, git_ref, path @ ..] if mode == "tree" || mode == "blob" => {
                let mut path = path.to_vec();
                if mode == "blob" {
                    if path.last().map(String::as_str) != Some("SKILL.md") {
                        return Err(refuse());
                    }
                    path.pop();
                }
                (git_ref.clone(), path.join("/"), None)
            }
            _ => return Err(refuse()),
        }
    };
    if !ref_ok(&git_ref) || (!path.is_empty() && safe_path(&path).is_none()) {
        return Err(refuse());
    }
    Ok(GitSource {
        kind,
        owner: owner.clone(),
        repo: repo.to_owned(),
        git_ref,
        path,
        skill,
        url: raw.chars().take(500).collect(),
    })
}

#[derive(Deserialize)]
struct Tree {
    #[serde(default)]
    tree: Vec<TreeEntry>,
    #[serde(default)]
    truncated: bool,
}

#[derive(Deserialize)]
struct TreeEntry {
    path: String,
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    mode: String,
    #[serde(default)]
    size: Option<u64>,
}

/// The URL `base` + `segments`, each segment percent-encoded.
fn joined(base: &Url, segments: &[&str]) -> Result<Url, String> {
    let mut url = base.clone();
    url.path_segments_mut()
        .map_err(|_| "The GitHub address is not valid.")?
        .pop_if_empty()
        .extend(segments);
    Ok(url)
}

fn failure(error: &OutboundError) -> String {
    format!("GitHub could not be read: {error}.")
}

fn status_failure(status: u16, what: &str) -> String {
    match status {
        301 | 302 | 307 | 308 => format!(
            "GitHub answered with a redirect for {what}: the repository may have moved. Use its new address."
        ),
        404 => {
            format!("{what} was not found on GitHub. Only public repositories can be installed.")
        }
        403 | 429 => "GitHub refused the request (rate limit). Try again later.".to_owned(),
        other => format!("GitHub answered HTTP {other} for {what}."),
    }
}

/// The folder of the skill in the repository's listing.
fn skill_folder(source: &GitSource, tree: &[TreeEntry]) -> Result<String, String> {
    let below = |path: &str| {
        source.path.is_empty()
            || path == source.path
            || path.starts_with(&format!("{}/", source.path))
    };
    let Some(skill) = &source.skill else {
        let wanted = if source.path.is_empty() {
            "SKILL.md".to_owned()
        } else {
            format!("{}/SKILL.md", source.path)
        };
        return if tree
            .iter()
            .any(|entry| entry.kind == "blob" && entry.path == wanted)
        {
            Ok(source.path.clone())
        } else {
            Err(format!(
                "No SKILL.md in {}. Link the skill's folder (the one with SKILL.md).",
                if source.path.is_empty() {
                    "the repository's top"
                } else {
                    &source.path
                }
            ))
        };
    };
    let mut found: Vec<String> = tree
        .iter()
        .filter(|entry| entry.kind == "blob")
        .filter_map(|entry| {
            let folder = if entry.path == "SKILL.md" {
                ""
            } else {
                entry.path.strip_suffix("/SKILL.md")?
            };
            let named = folder
                .rsplit('/')
                .next()
                .filter(|last| !last.is_empty())
                .unwrap_or(&source.repo);
            (named == skill && below(folder)).then(|| folder.to_owned())
        })
        .collect();
    found.sort_by_key(|folder| (folder.matches('/').count(), folder.clone()));
    found.into_iter().next().ok_or_else(|| {
        format!(
            "No folder named {skill} with a SKILL.md in {}/{}.",
            source.owner, source.repo
        )
    })
}

/// Downloads a skill from public GitHub with the guarded client (`super::outbound`): HTTPS to
/// public addresses only, no redirects, no proxy, time and size limits. The listing gives every
/// size first, so a too large skill fails before any file is downloaded.
pub async fn fetch(
    outbound: &Outbound,
    sources: &SkillSources,
    source: &GitSource,
) -> Result<SkillPackage, String> {
    let mut tree_url = joined(
        &sources.api,
        &[
            "repos",
            &source.owner,
            &source.repo,
            "git",
            "trees",
            &source.git_ref,
        ],
    )?;
    tree_url.set_query(Some("recursive=1"));
    let (status, bytes) = outbound
        .get_bytes(&tree_url, MAX_TREE_BYTES)
        .await
        .map_err(|error| failure(&error))?;
    if status != 200 {
        return Err(status_failure(
            status,
            &format!("{}/{}", source.owner, source.repo),
        ));
    }
    let tree: Tree =
        serde_json::from_slice(&bytes).map_err(|_| "GitHub answered in an unexpected way.")?;
    let folder = match skill_folder(source, &tree.tree) {
        Err(_) if tree.truncated => {
            return Err(
                "The repository is too large to list. Link the skill's folder (…/tree/<branch>/<folder>).".to_owned(),
            );
        }
        other => other?,
    };
    let prefix = if folder.is_empty() {
        String::new()
    } else {
        format!("{folder}/")
    };
    let mut wanted = Vec::new();
    let mut total = 0u64;
    for entry in &tree.tree {
        let Some(relative) = entry.path.strip_prefix(&prefix) else {
            continue;
        };
        // A nested skill is its own skill.
        if entry.kind == "tree" || entry.kind == "commit" || relative.is_empty() {
            continue;
        }
        if entry.mode == "120000" {
            return Err(format!(
                "{relative} is a symbolic link; skills may not have links."
            ));
        }
        if safe_path(relative).is_none() {
            return Err(format!("The path {relative} is not allowed."));
        }
        let size = entry.size.unwrap_or(0);
        let limit = if relative == "SKILL.md" {
            MAX_SKILL_MD_BYTES
        } else {
            MAX_FILE_BYTES
        };
        if size > limit as u64 {
            return Err(format!("{relative} is too large."));
        }
        total += size;
        wanted.push(relative.to_owned());
    }
    if wanted.len() > MAX_FILES + 1 {
        return Err(format!(
            "A skill may have at most {MAX_FILES} files besides SKILL.md."
        ));
    }
    if total > MAX_TOTAL_BYTES as u64 {
        return Err("A skill may have at most 2 MiB of files.".to_owned());
    }
    let mut skill_md = None;
    let mut files = Vec::new();
    for relative in wanted {
        let full = format!("{prefix}{relative}");
        let mut segments = vec![source.owner.as_str(), source.repo.as_str()];
        segments.extend(source.git_ref.split('/'));
        segments.extend(full.split('/'));
        let url = joined(&sources.raw, &segments)?;
        let limit = if relative == "SKILL.md" {
            MAX_SKILL_MD_BYTES
        } else {
            MAX_FILE_BYTES
        };
        let (status, content) = outbound
            .get_bytes(&url, limit)
            .await
            .map_err(|error| failure(&error))?;
        if status != 200 {
            return Err(status_failure(status, &relative));
        }
        if relative == "SKILL.md" {
            skill_md = Some(content);
        } else {
            files.push(SkillFile {
                path: relative,
                content,
            });
        }
    }
    package(&skill_md.ok_or("The folder has no SKILL.md.")?, files)
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use zip::write::SimpleFileOptions;

    use super::*;

    fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, content) in entries {
            writer
                .start_file(*name, SimpleFileOptions::default())
                .unwrap();
            writer.write_all(content).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    const SKILL: &[u8] = b"---\nname: pdf-forms\ndescription: Fill PDF forms. Use when the driver sends a form.\n---\n# PDF forms\n\nRun scripts/fill.sh.\n";

    #[test]
    fn names_follow_the_spec() {
        for good in ["pdf", "pdf-forms", "a1-b2"] {
            assert_eq!(checked_name(good).unwrap(), good);
        }
        for bad in [
            "",
            "PDF",
            "-pdf",
            "pdf-",
            "pdf--forms",
            "pdf_forms",
            "pdf forms",
            &"a".repeat(65),
        ] {
            assert!(checked_name(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn front_matter_reads_plain_quoted_and_block_values() {
        let (name, description, body) = parse_skill_md(
            "---\nname: \"brand-voice\"\nlicense: MIT\nmetadata:\n  author: someone\n  version: \"1\"\ndescription: >\n  Write in the brand voice.\n  Use for blog posts.\n---\n\nBody line\n",
        )
        .unwrap();
        assert_eq!(name, "brand-voice");
        assert_eq!(description, "Write in the brand voice. Use for blog posts.");
        assert_eq!(body, "Body line");
        let (_, description, _) =
            parse_skill_md("---\r\nname: x\r\ndescription: 'It''s short'\r\n---\r\nB").unwrap();
        assert_eq!(description, "It's short");
        assert!(parse_skill_md("# no front matter").is_err());
        assert!(parse_skill_md("---\nname: x\n").is_err());
        assert!(
            parse_skill_md("---\nname: x\n---\nbody").is_err(),
            "no description"
        );
    }

    #[test]
    fn a_zip_with_skill_md_at_the_top_or_in_one_folder_is_read() {
        let package = from_zip(&zip_of(&[
            ("SKILL.md", SKILL),
            ("scripts/fill.sh", b"echo hi"),
        ]))
        .unwrap();
        assert_eq!(package.name, "pdf-forms");
        assert_eq!(
            package.files,
            vec![SkillFile {
                path: "scripts/fill.sh".into(),
                content: b"echo hi".to_vec()
            }]
        );
        let package = from_zip(&zip_of(&[
            ("pdf-forms/SKILL.md", SKILL),
            ("pdf-forms/references/fields.md", b"x"),
            ("__MACOSX/pdf-forms/._SKILL.md", b"junk"),
        ]))
        .unwrap();
        assert_eq!(package.files.len(), 1);
        assert_eq!(package.files[0].path, "references/fields.md");
        assert!(from_zip(&zip_of(&[("README.md", b"x")])).is_err());
        assert!(from_zip(b"not a zip").is_err());
    }

    #[test]
    fn a_zip_that_could_leave_the_folder_is_refused() {
        for evil in [
            "../evil.sh",
            "/etc/passwd",
            "a/../../b",
            "a\\..\\b",
            "C:/x",
            "./x",
        ] {
            let error = from_zip(&zip_of(&[("SKILL.md", SKILL), (evil, b"x")])).unwrap_err();
            assert!(error.contains("not allowed"), "{evil}: {error}");
        }
    }

    #[test]
    fn a_zip_with_a_symbolic_link_is_refused() {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        writer
            .start_file("SKILL.md", SimpleFileOptions::default())
            .unwrap();
        writer.write_all(SKILL).unwrap();
        writer
            .add_symlink(
                "scripts/secret",
                "/etc/shadow",
                SimpleFileOptions::default(),
            )
            .unwrap();
        let bytes = writer.finish().unwrap().into_inner();
        assert!(from_zip(&bytes).unwrap_err().contains("symbolic link"));
    }

    #[test]
    fn sizes_and_counts_are_limited() {
        let big = vec![b'a'; MAX_FILE_BYTES + 1];
        assert!(
            from_zip(&zip_of(&[("SKILL.md", SKILL), ("big.txt", &big)]))
                .unwrap_err()
                .contains("too large")
        );
        let names: Vec<String> = (0..=MAX_FILES).map(|i| format!("f{i}.txt")).collect();
        let mut entries: Vec<(&str, &[u8])> = vec![("SKILL.md", SKILL)];
        entries.extend(names.iter().map(|name| (name.as_str(), b"x" as &[u8])));
        assert!(
            from_zip(&zip_of(&entries))
                .unwrap_err()
                .contains("at most 100 files")
        );
        let part = vec![b'a'; MAX_FILE_BYTES];
        let parts: Vec<String> = (0..5).map(|i| format!("p{i}.txt")).collect();
        let mut entries: Vec<(&str, &[u8])> = vec![("SKILL.md", SKILL)];
        entries.extend(parts.iter().map(|name| (name.as_str(), part.as_slice())));
        let bytes = zip_of(&entries);
        assert!(bytes.len() < MAX_ZIP_BYTES);
        assert!(from_zip(&bytes).unwrap_err().contains("2 MiB"));
        assert!(
            from_zip(&vec![0; MAX_ZIP_BYTES + 1])
                .unwrap_err()
                .contains("4 MiB")
        );
    }

    #[test]
    fn sources_are_read_from_github_skills_sh_and_short_forms() {
        let tree =
            parse_source("https://github.com/acme/skills/tree/main/skills/pdf-forms").unwrap();
        assert_eq!(
            (
                tree.kind,
                tree.owner.as_str(),
                tree.repo.as_str(),
                tree.git_ref.as_str(),
                tree.path.as_str(),
                tree.skill.clone()
            ),
            ("github", "acme", "skills", "main", "skills/pdf-forms", None)
        );
        let blob = parse_source("https://github.com/acme/skills/blob/v1/pdf/SKILL.md").unwrap();
        assert_eq!((blob.git_ref.as_str(), blob.path.as_str()), ("v1", "pdf"));
        let repo = parse_source("https://github.com/acme/one-skill.git").unwrap();
        assert_eq!(
            (
                repo.repo.as_str(),
                repo.git_ref.as_str(),
                repo.path.as_str()
            ),
            ("one-skill", "HEAD", "")
        );
        let sh = parse_source("https://skills.sh/acme/skills/pdf-forms").unwrap();
        assert_eq!(
            (sh.kind, sh.skill.as_deref()),
            ("skills_sh", Some("pdf-forms"))
        );
        assert_eq!(
            parse_source("acme/skills@pdf-forms")
                .unwrap()
                .skill
                .as_deref(),
            Some("pdf-forms")
        );
        assert_eq!(
            parse_source("acme/skills/skills/pdf").unwrap().path,
            "skills/pdf"
        );
        for bad in [
            "http://github.com/a/b",
            "https://evil.example/a/b",
            "https://github.com:8443/a/b",
            "https://user@github.com/a/b",
            "https://github.com/a",
            "a/b/../x",
            "https://github.com/a/b/tree/..%2f/x",
            "https://skills.sh/a/b",
            "a",
        ] {
            assert!(parse_source(bad).is_err(), "{bad}");
        }
    }
}
