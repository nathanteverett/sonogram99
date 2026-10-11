//! Renderer preference and diagnostics. Selecting software never initializes a GPU.

use sonograph_core::acceleration;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Preference {
    #[default]
    Auto,
    Software,
}

impl Preference {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value.trim() {
            "auto" => Ok(Self::Auto),
            "software" => Ok(Self::Software),
            _ => Err("Rendering takes auto or software.".into()),
        }
    }
    fn value(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Software => "software",
        }
    }
    pub fn load(path: &Path) -> Result<Self, String> {
        match std::fs::read_to_string(path) {
            Ok(value) => Self::parse(&value),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::Auto),
            Err(e) => Err(format!("Could not load rendering preference: {e}")),
        }
    }
    pub fn save(self, path: &Path) -> Result<(), String> {
        let parent = path
            .parent()
            .ok_or("Rendering settings have no parent directory")?;
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
        std::fs::write(&tmp, format!("{}\n", self.value())).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, path).map_err(|e| format!("Could not save rendering preference: {e}"))
    }
}

pub fn config_path() -> Option<PathBuf> {
    let root = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".config")));
    root.map(|p| p.join("Sonograph99").join("rendering.conf"))
}

pub fn apply(preference: Preference) {
    acceleration::software(None);
    if preference == Preference::Auto {
        match sonograph_gpu::Gpu::new() {
            Ok(gpu) => acceleration::install(Box::new(gpu)),
            Err(reason) => acceleration::software(Some(reason)),
        }
    }
    eprintln!("{}", description());
}

pub fn description() -> String {
    let status = acceleration::status();
    match (status.adapter, status.fallback) {
        (Some(adapter), _) => format!("GPU acceleration: {adapter}"),
        (_, Some(reason)) => format!("Software fallback: {reason}"),
        _ => "Software rendering".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rendering_preference_defaults_validates_and_round_trips() {
        let dir = std::env::temp_dir().join(format!("sonograph-rendering-{}", std::process::id()));
        let path = dir.join("rendering.conf");
        let _ = std::fs::remove_file(&path);
        assert_eq!(Preference::load(&path).unwrap(), Preference::Auto);
        Preference::Software.save(&path).unwrap();
        assert_eq!(Preference::load(&path).unwrap(), Preference::Software);
        Preference::Auto.save(&path).unwrap();
        assert_eq!(Preference::load(&path).unwrap(), Preference::Auto);
        assert!(Preference::parse("gpu-only").is_err());
        std::fs::remove_file(path).unwrap();
        let _ = std::fs::remove_dir(dir);
    }
    #[test]
    fn software_choice_does_not_create_a_device() {
        apply(Preference::Software);
        assert_eq!(description(), "Software rendering");
        assert_eq!(acceleration::status(), acceleration::Status::default());
    }
}
