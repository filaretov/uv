//! Create a bare virtualenv without any packages install

use std::io;
use std::io::{BufWriter, Write};

use camino::{FromPathError, Utf8Path, Utf8PathBuf};
use fs_err as fs;
#[cfg(unix)]
use fs_err::os::unix::fs::symlink;
use fs_err::File;
use tracing::info;

use puffin_interpreter::Interpreter;

/// The bash activate scripts with the venv dependent paths patches out
const ACTIVATE_TEMPLATES: &[(&str, &str)] = &[
    ("activate", include_str!("activator/activate")),
    ("activate.csh", include_str!("activator/activate.csh")),
    ("activate.fish", include_str!("activator/activate.fish")),
    ("activate.nu", include_str!("activator/activate.nu")),
    ("activate.ps1", include_str!("activator/activate.ps1")),
    (
        "activate_this.py",
        include_str!("activator/activate_this.py"),
    ),
];
const VIRTUALENV_PATCH: &str = include_str!("_virtualenv.py");

/// Very basic `.cfg` file format writer.
fn write_cfg(f: &mut impl Write, data: &[(&str, String); 8]) -> io::Result<()> {
    for (key, value) in data {
        writeln!(f, "{key} = {value}")?;
    }
    Ok(())
}

/// Absolute paths of the virtualenv
#[derive(Debug)]
pub struct VenvPaths {
    /// The location of the virtualenv, e.g. `.venv`
    #[allow(unused)]
    pub root: Utf8PathBuf,

    /// The python interpreter.rs inside the virtualenv, on unix `.venv/bin/python`
    #[allow(unused)]
    pub interpreter: Utf8PathBuf,

    /// The directory with the scripts, on unix `.venv/bin`
    #[allow(unused)]
    pub bin: Utf8PathBuf,

    /// The site-packages directory where all the packages are installed to, on unix
    /// and python 3.11 `.venv/lib/python3.11/site-packages`
    #[allow(unused)]
    pub site_packages: Utf8PathBuf,
}

/// Write all the files that belong to a venv without any packages installed.
pub fn create_bare_venv(location: &Utf8Path, interpreter: &Interpreter) -> io::Result<VenvPaths> {
    let base_python: &Utf8Path = interpreter
        .sys_executable()
        .try_into()
        .map_err(|err: FromPathError| err.into_io_error())?;
    if location.exists() {
        if location.join("pyvenv.cfg").is_file() {
            info!("Removing existing directory");
            fs::remove_dir_all(location)?;
        } else {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("The directory {location} exists, but it is not virtualenv"),
            ));
        }
    }
    fs::create_dir_all(location)?;
    // TODO: I bet on windows we'll have to strip the prefix again
    let location = location.canonicalize_utf8()?;
    let bin_dir = {
        #[cfg(unix)]
        {
            location.join("bin")
        }
        #[cfg(windows)]
        {
            location.join("Bin")
        }
        #[cfg(not(any(unix, windows)))]
        {
            compile_error!("only unix (like mac and linux) and windows are supported")
        }
    };

    fs::write(location.join(".gitignore"), "*")?;

    // Different names for the python interpreter
    fs::create_dir(&bin_dir)?;
    let venv_python = {
        #[cfg(unix)]
        {
            bin_dir.join("python")
        }
        #[cfg(windows)]
        {
            bin_dir.join("python.exe")
        }
        #[cfg(not(any(unix, windows)))]
        {
            compile_error!("only unix (like mac and linux) and windows are supported")
        }
    };
    #[cfg(unix)]
    {
        symlink(base_python, &venv_python)?;
        symlink(
            "python",
            bin_dir.join(format!("python{}", interpreter.simple_version().0)),
        )?;
        symlink(
            "python",
            bin_dir.join(format!(
                "python{}.{}",
                interpreter.simple_version().0,
                interpreter.simple_version().1
            )),
        )?;
    }

    // Add all the activate scripts for different shells
    for (name, template) in ACTIVATE_TEMPLATES {
        let activator = template
            .replace("{{ VIRTUAL_ENV_DIR }}", location.as_str())
            .replace(
                "{{ RELATIVE_SITE_PACKAGES }}",
                &format!(
                    "../lib/python{}.{}/site-packages",
                    interpreter.simple_version().0,
                    interpreter.simple_version().1
                ),
            );
        fs::write(bin_dir.join(name), activator)?;
    }

    // pyvenv.cfg
    let python_home = base_python
        .parent()
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "The python interpreter needs to have a parent directory",
            )
        })?
        .to_string();
    let pyvenv_cfg_data = &[
        ("home", python_home),
        ("implementation", "CPython".to_string()),
        (
            "version_info",
            interpreter.markers().python_version.string.clone(),
        ),
        ("gourgeist", env!("CARGO_PKG_VERSION").to_string()),
        // I wouldn't allow this option anyway
        ("include-system-site-packages", "false".to_string()),
        (
            "base-prefix",
            interpreter.base_prefix().to_string_lossy().to_string(),
        ),
        (
            "base-exec-prefix",
            interpreter.base_exec_prefix().to_string_lossy().to_string(),
        ),
        ("base-executable", base_python.to_string()),
    ];
    let mut pyvenv_cfg = BufWriter::new(File::create(location.join("pyvenv.cfg"))?);
    write_cfg(&mut pyvenv_cfg, pyvenv_cfg_data)?;
    drop(pyvenv_cfg);

    // TODO: This is different on windows
    let site_packages = location
        .join("lib")
        .join(format!(
            "python{}.{}",
            interpreter.simple_version().0,
            interpreter.simple_version().1
        ))
        .join("site-packages");
    fs::create_dir_all(&site_packages)?;
    // Install _virtualenv.py patch.
    // Frankly no idea what that does, i just copied it from virtualenv knowing that
    // distutils/setuptools will have their cursed reasons
    fs::write(site_packages.join("_virtualenv.py"), VIRTUALENV_PATCH)?;
    fs::write(site_packages.join("_virtualenv.pth"), "import _virtualenv")?;

    Ok(VenvPaths {
        root: location.to_path_buf(),
        interpreter: venv_python,
        bin: bin_dir,
        site_packages,
    })
}
