use std::fmt::Write;
use std::path::PathBuf;

use anyhow::Result;

use owo_colors::OwoColorize;
use pep508_rs::PackageName;
use uv_cache::Cache;
use uv_client::{BaseClientBuilder, Connectivity};
use uv_configuration::PreviewMode;
use uv_distribution::pyproject_mut::PyProjectTomlMut;
use uv_distribution::{ProjectWorkspace, WorkspaceError};
use uv_fs::Simplified;
use uv_python::{
    EnvironmentPreference, PythonFetch, PythonInstallation, PythonPreference, PythonRequest,
};
use uv_warnings::warn_user_once;

use crate::commands::ExitStatus;
use crate::printer::Printer;

/// Add one or more packages to the project requirements.
#[allow(clippy::single_match_else)]
pub(crate) async fn init(
    path: Option<String>,
    name: Option<PackageName>,
    no_readme: bool,
    no_pin: bool,
    python: Option<String>,
    python_preference: PythonPreference,
    python_fetch: PythonFetch,
    preview: PreviewMode,
    connectivity: Connectivity,
    native_tls: bool,
    cache: &Cache,
    printer: Printer,
) -> Result<ExitStatus> {
    if preview.is_disabled() {
        warn_user_once!("`uv init` is experimental and may change without warning.");
    }

    // Discover the current workspace, if it exists.
    let current_dir = std::env::current_dir()?.canonicalize()?;
    let workspace = match ProjectWorkspace::discover(&current_dir, None).await {
        Ok(project) => Some(project),
        Err(WorkspaceError::MissingPyprojectToml) => None,
        Err(err) => return Err(err.into()),
    };

    let project_dir = match path {
        None => current_dir.clone(),
        Some(ref path) => PathBuf::from(path),
    };

    let name = match name {
        Some(name) => name,
        None => {
            // Get the name of the directory.
            let name = project_dir
                .file_name()
                .and_then(|path| path.to_str())
                .expect("Invalid package name");
            PackageName::new(name.to_string())?
        }
    };

    // Make sure the package does not already exist.
    if project_dir.join("pyproject.toml").exists() {
        anyhow::bail!("Package is already initialized")
    }

    // Create the directory for the project.
    let src_dir = project_dir.join("src").join(name.as_ref());
    fs_err::create_dir_all(&src_dir)?;

    // Create the `pyproject.toml`.
    fs_err::write(
        project_dir.join("pyproject.toml"),
        indoc::formatdoc! {r#"
        [project]
        name = "{name}"
        version = "0.1.0"
        description = "Add your description here"
        dependencies = []
        readme = "README.md"

        [tool.uv]
        dev-dependencies = []
    "#},
    )?;

    // Create `src/{name}/__init__.py`.
    let init_py = src_dir.join("__init__.py");
    // Avoid overwriting existing content.
    if !init_py.try_exists()? {
        fs_err::write(
            init_py,
            indoc::formatdoc! {r#"
            def hello() -> str:
                return "Hello from {name}!"
            "#},
        )?;
    }

    // Create the `README.md`.
    if !no_readme {
        let readme = project_dir.join("README.md");
        // Avoid overwriting existing content.
        if !readme.exists() {
            fs_err::write(readme, String::new())?;
        }
    }

    // Create `.python-version` file if we aren't already in a workspace.
    if !no_pin && workspace.is_none() {
        let client_builder = BaseClientBuilder::default()
            .connectivity(connectivity)
            .native_tls(native_tls);

        // Find the python version.
        let interpreter = PythonInstallation::find_or_fetch(
            python.as_deref().map(PythonRequest::parse),
            EnvironmentPreference::OnlySystem,
            python_preference,
            python_fetch,
            &client_builder,
            cache,
        )
        .await?
        .into_interpreter();

        // Write the python version to `.python-version`.
        fs_err::write(
            project_dir.join(".python-version"),
            interpreter.python_version().to_string(),
        )?;
    }

    if let Some(workspace) = workspace {
        // Add the package to the workspace.
        let mut pyproject =
            PyProjectTomlMut::from_toml(workspace.current_project().pyproject_toml())?;
        pyproject.add_workspace(project_dir.to_string_lossy().to_string())?;

        // Save the modified `pyproject.toml`.
        fs_err::write(
            workspace.current_project().root().join("pyproject.toml"),
            pyproject.to_string(),
        )?;

        writeln!(
            printer.stderr(),
            "Adding {} as member of workspace {}",
            name.cyan(),
            current_dir.simplified_display().cyan()
        )?;
    }

    match path {
        Some(_) => writeln!(
            printer.stderr(),
            "Initialized project {} in {}",
            name.cyan(),
            project_dir
                .simple_canonicalize()
                .unwrap_or_else(|_| project_dir.simplified().to_path_buf())
                .display()
                .cyan()
        )?,
        None => writeln!(printer.stderr(), "Initialized project {}", name.cyan())?,
    }

    Ok(ExitStatus::Success)
}
