//! Data-layer store for the project registry.
//!
//! `manifest_store` owns the disk: it persists the project registry
//! (workspace plus display name) to a local JSON file. The serde DTOs
//! are private to this edge; the domain types in `manifest_domain`
//! stay serde-free and cross the boundary through conversion here.

mod error;

pub use error::Error;

use manifest_domain::{ProjectId, ProjectSummary};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// On-disk format version this Manifest build reads and writes.
const FORMAT_VERSION: u32 = 1;

/// Private on-disk shape of the whole registry file.
///
/// Unknown fields at the top level and on entries are ignored on
/// load, so a newer writer adding fields never breaks this reader.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct RegistryFile {
    /// Format version; must equal [`FORMAT_VERSION`].
    version: u32,
    /// Projects in display order.
    projects: Vec<ProjectEntry>,
}

/// Private on-disk shape of one registry entry.
///
/// Unknown fields are ignored on load (serde default); only
/// `workspace` and `name` are read.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct ProjectEntry {
    /// Project folder; the registry key, unique per file.
    workspace: String,
    /// Short human name shown in the project list.
    name: String,
}

/// What `load` found on disk.
///
/// [`Loaded::Missing`] (no file at all, so the caller may seed one)
/// and [`Loaded::Present`] with zero projects (a file exists that
/// lists no projects) are different situations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Loaded {
    /// No registry file exists at the store path.
    Missing,
    /// The registry file exists and lists these projects in file order.
    Present(Vec<ProjectSummary>),
}

/// Disk-backed store for the project registry.
#[derive(Debug)]
pub struct ProjectStore {
    /// Registry file location.
    path: PathBuf,
}

impl ProjectStore {
    /// Build a store for the registry file at `path`.
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// Build a store at the platform data directory.
    ///
    /// Returns `None` when no project directories exist (for example
    /// when no home directory is set). On Linux the path is
    /// `~/.local/share/manifest/projects.json`.
    pub fn from_env() -> Option<Self> {
        let dirs = directories::ProjectDirs::from("", "", "manifest")?;
        Some(Self::new(dirs.data_dir().join("projects.json")))
    }

    /// Borrow the registry file location.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Read the registry file.
    ///
    /// A missing file yields [`Loaded::Missing`]; anything else wrong
    /// with the read or the contents yields [`Error`]. Loading never
    /// writes to or modifies the file.
    pub fn load(&self) -> Result<Loaded, Error> {
        let text = match std::fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Loaded::Missing);
            }
            Err(source) => {
                return Err(Error::Io {
                    path: self.path.clone(),
                    source,
                });
            }
        };
        let file: RegistryFile = serde_json::from_str(&text).map_err(|err| Error::Corrupt {
            path: self.path.clone(),
            detail: err.to_string(),
        })?;
        if file.version != FORMAT_VERSION {
            return Err(Error::UnsupportedVersion {
                path: self.path.clone(),
                found: file.version,
            });
        }
        let mut seen: HashSet<&str> = HashSet::new();
        let mut projects = Vec::with_capacity(file.projects.len());
        for entry in &file.projects {
            // Blankness checks trim, but the stored strings stay verbatim
            // so a save/load round trip preserves exactly what was saved.
            if entry.workspace.trim().is_empty() {
                return Err(Error::Corrupt {
                    path: self.path.clone(),
                    detail: String::from("a project entry has a blank workspace"),
                });
            }
            if entry.name.trim().is_empty() {
                return Err(Error::Corrupt {
                    path: self.path.clone(),
                    detail: format!(
                        "project at workspace {} has a blank name",
                        entry.workspace.trim()
                    ),
                });
            }
            if !seen.insert(entry.workspace.as_str()) {
                return Err(Error::Corrupt {
                    path: self.path.clone(),
                    detail: format!("project at workspace {} is listed twice", entry.workspace),
                });
            }
            projects.push(ProjectSummary {
                id: ProjectId::for_workspace(&entry.workspace),
                name: entry.name.clone(),
                workspace: Some(entry.workspace.clone()),
            });
        }
        Ok(Loaded::Present(projects))
    }

    /// Persist `projects` to the registry file.
    ///
    /// Validation runs before any file is touched: a project without
    /// a usable workspace, with a blank name, or sharing a workspace
    /// with another entry yields [`Error::Invalid`] and leaves the
    /// file as it was. The write goes to a sibling temp file first
    /// and is renamed into place, so a crash never leaves a
    /// half-written registry. Entry order is preserved exactly.
    pub fn save(&self, projects: &[ProjectSummary]) -> Result<(), Error> {
        validate(projects)?;
        if let Some(parent) = self.path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).map_err(|source| Error::Io {
                path: self.path.clone(),
                source,
            })?;
        }
        let file = RegistryFile {
            version: FORMAT_VERSION,
            projects: projects
                .iter()
                .map(|project| ProjectEntry {
                    // Validation above guarantees a non-blank workspace.
                    workspace: project.workspace.clone().unwrap_or_default(),
                    name: project.name.clone(),
                })
                .collect(),
        };
        let mut text = serde_json::to_string_pretty(&file).map_err(|err| Error::Invalid {
            detail: err.to_string(),
        })?;
        text.push('\n');
        let temp = self.path.with_extension("json.tmp");
        let write_result = std::fs::write(&temp, text)
            .map_err(|source| Error::Io {
                path: temp.clone(),
                source,
            })
            .and_then(|()| {
                std::fs::rename(&temp, &self.path).map_err(|source| Error::Io {
                    path: self.path.clone(),
                    source,
                })
            });
        if write_result.is_err() {
            let _ = std::fs::remove_file(&temp);
        }
        write_result
    }
}

/// Check `projects` for anything `save` cannot persist.
fn validate(projects: &[ProjectSummary]) -> Result<(), Error> {
    let mut seen: HashSet<&str> = HashSet::new();
    for project in projects {
        let workspace = project.workspace.as_deref().unwrap_or("").trim();
        if workspace.is_empty() {
            return Err(Error::Invalid {
                detail: format!("project {} has no workspace to save", describe(project),),
            });
        }
        if project.name.trim().is_empty() {
            return Err(Error::Invalid {
                detail: format!("project at workspace {workspace} has a blank name"),
            });
        }
        if !seen.insert(workspace) {
            return Err(Error::Invalid {
                detail: format!("project at workspace {workspace} is listed twice"),
            });
        }
    }
    Ok(())
}

/// Name a project for error text: its name, else its id handle.
fn describe(project: &ProjectSummary) -> String {
    if project.name.trim().is_empty() {
        format!("`{}`", project.id.as_str())
    } else {
        format!("`{}`", project.name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// Unique temp dir per test: temp dir + process id + atomic counter
    /// + test-specific suffix. Best-effort cleanup after the test.
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_path(suffix: &str) -> PathBuf {
        let unique = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "manifest-store-test-{}-{unique}-{suffix}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("create unique temp dir");
        dir.join("projects.json")
    }

    fn cleanup(path: &Path) {
        if let Some(parent) = path.parent() {
            let _ = std::fs::remove_dir_all(parent);
        }
    }

    fn helper_project(workspace: &str, name: &str) -> ProjectSummary {
        ProjectSummary {
            id: ProjectId::for_workspace(workspace),
            name: String::from(name),
            workspace: Some(String::from(workspace)),
        }
    }

    #[test]
    fn load_missing_file_reports_missing() {
        let path = temp_path("missing");
        let store = ProjectStore::new(path.clone());
        assert_eq!(store.load().expect("load succeeds"), Loaded::Missing);
        cleanup(&path);
    }

    #[test]
    fn save_then_load_round_trips_order_names_workspaces_and_ids() {
        let path = temp_path("round-trip");
        let store = ProjectStore::new(path.clone());
        let projects = vec![
            helper_project("/home/dev/beta", "beta"),
            helper_project("/home/dev/alpha", "alpha"),
        ];
        store.save(&projects).expect("save succeeds");
        let loaded = store.load().expect("load succeeds");
        assert_eq!(loaded, Loaded::Present(projects.clone()));
        if let Loaded::Present(got) = loaded {
            assert_eq!(got[0].name, "beta");
            assert_eq!(got[0].workspace, Some(String::from("/home/dev/beta")));
            assert_eq!(got[0].id, ProjectId::for_workspace("/home/dev/beta"));
            assert_eq!(got[1].id, ProjectId::for_workspace("/home/dev/alpha"));
        } else {
            panic!("expected Present");
        }
        cleanup(&path);
    }

    #[test]
    fn padded_name_survives_save_load_round_trip_verbatim() {
        let path = temp_path("padded-round-trip");
        let store = ProjectStore::new(path.clone());
        let projects = vec![helper_project("/w", " beta ")];
        store.save(&projects).expect("save succeeds");
        let loaded = store.load().expect("load succeeds");
        assert_eq!(loaded, Loaded::Present(projects.clone()));
        if let Loaded::Present(got) = loaded {
            assert_eq!(got[0].name, " beta ");
        } else {
            panic!("expected Present");
        }
        cleanup(&path);
    }

    #[test]
    fn save_creates_missing_parent_directories() {
        let path = temp_path("parents");
        let nested = path
            .parent()
            .expect("temp path has a parent")
            .join("deep")
            .join("nested")
            .join("projects.json");
        ProjectStore::new(nested.clone())
            .save(&[helper_project("/home/dev/beta", "beta")])
            .expect("save creates parents");
        assert!(nested.exists(), "registry file exists under new parents");
        cleanup(&path);
    }

    #[test]
    fn save_empty_list_loads_present_empty_not_missing() {
        let path = temp_path("empty");
        let store = ProjectStore::new(path.clone());
        store.save(&[]).expect("save empty succeeds");
        let loaded = store.load().expect("load succeeds");
        assert_eq!(loaded, Loaded::Present(vec![]));
        assert_ne!(loaded, Loaded::Missing);
        cleanup(&path);
    }

    #[test]
    fn corrupt_json_reports_corrupt_and_leaves_bytes_untouched() {
        let path = temp_path("corrupt");
        std::fs::write(&path, "{ not valid json }\n").expect("write corrupt file");
        let before = std::fs::read(&path).expect("read corrupt file");
        let err = ProjectStore::new(path.clone())
            .load()
            .expect_err("corrupt file fails to load");
        assert!(matches!(err, Error::Corrupt { .. }), "got {err:?}");
        let after = std::fs::read(&path).expect("read corrupt file again");
        assert_eq!(before, after, "load never modifies the file");
        cleanup(&path);
    }

    #[test]
    fn version_two_file_reports_unsupported_version() {
        let path = temp_path("version");
        std::fs::write(
            &path,
            "{\"version\": 2, \"projects\": [{\"workspace\": \"/w\", \"name\": \"w\"}]}\n",
        )
        .expect("write version 2 file");
        let err = ProjectStore::new(path.clone())
            .load()
            .expect_err("version 2 fails to load");
        match err {
            Error::UnsupportedVersion { found, .. } => assert_eq!(found, 2),
            other => panic!("expected UnsupportedVersion, got {other:?}"),
        }
        cleanup(&path);
    }

    #[test]
    fn unknown_extra_fields_are_ignored() {
        let path = temp_path("extra");
        std::fs::write(
            &path,
            "{\"version\": 1, \"extra\": true, \"projects\": [{\"workspace\": \"/home/dev/beta\", \"name\": \"beta\", \"nick\": \"b\"}]}\n",
        )
        .expect("write file with extra fields");
        let loaded = ProjectStore::new(path.clone())
            .load()
            .expect("extra fields are ignored");
        assert_eq!(
            loaded,
            Loaded::Present(vec![helper_project("/home/dev/beta", "beta")]),
        );
        cleanup(&path);
    }

    #[test]
    fn blank_name_blank_workspace_and_duplicates_are_corrupt() {
        for (suffix, entry) in [
            ("blank-name", "{\"workspace\": \"/w\", \"name\": \"  \"}"),
            (
                "blank-workspace",
                "{\"workspace\": \"   \", \"name\": \"w\"}",
            ),
            (
                "duplicate",
                "{\"workspace\": \"/w\", \"name\": \"a\"}, {\"workspace\": \"/w\", \"name\": \"b\"}",
            ),
        ] {
            let path = temp_path(suffix);
            std::fs::write(
                &path,
                format!("{{\"version\": 1, \"projects\": [{entry}]}}\n"),
            )
            .expect("write bad file");
            let err = ProjectStore::new(path.clone())
                .load()
                .expect_err("bad entries fail to load");
            assert!(
                matches!(err, Error::Corrupt { .. }),
                "{suffix}: got {err:?}"
            );
            cleanup(&path);
        }
    }

    #[test]
    fn save_rejects_unpersistable_projects_without_writing() {
        let rejected: Vec<Vec<ProjectSummary>> = vec![
            vec![ProjectSummary {
                id: ProjectId::new("project-flex"),
                name: String::from("flex"),
                workspace: None,
            }],
            vec![helper_project("   ", "blank-workspace")],
            vec![helper_project("/w", "   ")],
            vec![helper_project("/w", "a"), helper_project("/w", "b")],
        ];
        for (index, projects) in rejected.iter().enumerate() {
            let path = temp_path(&format!("invalid-{index}"));
            let err = ProjectStore::new(path.clone())
                .save(projects)
                .expect_err("unpersistable projects fail to save");
            assert!(matches!(err, Error::Invalid { .. }), "got {err:?}");
            assert!(!path.exists(), "no file is written on Invalid");
            cleanup(&path);
        }
    }

    #[test]
    fn second_save_overwrites_first() {
        let path = temp_path("overwrite");
        let store = ProjectStore::new(path.clone());
        store
            .save(&[helper_project("/home/dev/old", "old")])
            .expect("first save succeeds");
        let second = vec![
            helper_project("/home/dev/new-a", "new-a"),
            helper_project("/home/dev/new-b", "new-b"),
        ];
        store.save(&second).expect("second save succeeds");
        assert_eq!(
            store.load().expect("load succeeds"),
            Loaded::Present(second)
        );
        cleanup(&path);
    }

    #[test]
    fn from_env_points_at_manifest_projects_json() {
        let store = ProjectStore::from_env().expect("project dirs exist on Linux");
        assert_eq!(
            store.path().file_name().and_then(|name| name.to_str()),
            Some("projects.json"),
        );
        assert_eq!(
            store
                .path()
                .parent()
                .and_then(|parent| parent.file_name())
                .and_then(|name| name.to_str()),
            Some("manifest"),
        );
    }
}
