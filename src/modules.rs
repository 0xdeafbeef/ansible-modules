use anyhow::Error;
use serde::{Deserialize, Deserializer};
use std::collections::HashMap;
use std::fmt::{Debug, Display};
use std::fs::File;
use std::io::Read;
use std::net::ToSocketAddrs;
use std::path::{Path, PathBuf};
use toml::from_str;
use walkdir::{DirEntry, WalkDir};

enum ExecType {
    Bin,
    Python,
    Bash,
}
impl From<&str> for ExecType {
    fn from(a: &str) -> Self {
        match a.to_lowercase().as_str() {
            "bin" => Self::Bin,
            "bash" => Self::Bash,
            "sh" => Self::Bash,
            "py" => Self::Python,
            "python" => Self::Python,
            _ => panic!(format!("Bad module type is provided: {}", a)),
        }
    }
}

impl<'de> Deserialize<'de> for ExecType {
    fn deserialize<D>(deserializer: D) -> Result<Self, <D as Deserializer<'de>>::Error>
        where
            D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Ok(ExecType::from(s.as_str()))
    }
}

#[derive(Deserialize)]
pub struct ModuleProps {
    module_name: String,
    module_type: ExecType,
    exec_path: PathBuf,
}

enum ModuleContent{
    Shell( HashMap<String,String>),
    Binary(PathBuf),
    Python(String)
}

struct  Module{
    module_type: ExecType,
    module_content:ModuleContent
}

impl ModuleProps{
    fn check_filename(filename: &DirEntry) -> bool {
        let ext = match filename.path().extension() {
            Some(a) => a.to_string_lossy().to_lowercase(),
            None => return false,
        };
        if ext == "toml" {
            true
        } else {
            false
        }
    }

}

impl Module {

    pub fn new(path: &Path) -> Result<Module, Error> {
        let file_2_string =
            |p :&Path| ->Result<String, std::io::Error>
                {
                    let mut file = File::open(p)?;
                    let mut content = String::new();
                    file.read_to_string(&mut content)?;
                    Ok(content)
                };

        let res:ModuleProps = from_str(&file_2_string(path)?)?;

        let content =
            match res.module_type{
                ExecType::Bin =>ModuleContent::Binary( res.exec_path),
                ExecType::Python => ModuleContent::Python( file_2_string(&res.exec_path)?),
                ExecType::Bash=>{
                    let unparsed = file_2_string(&res.exec_path)?;
                    let table: HashMap<_,_>=from_str(&unparsed)?;
                    ModuleContent::Shell(table)
                }
            };
        Ok(Module{
            module_type: res.module_type,
            module_content: content
        })
    }

    pub fn execute<A>(&self, ip: A) ->Result<(), Error>
        where
            A: Display + ToSocketAddrs + Send + Sync + Clone + Debug + Eq + std::hash::Hash + ToString,
    {
        match self.module_type {
            ExecType::Bash => (),
            ExecType::Python => (),
            ExecType::Bin => (),
        };
        Ok(())
    }
}
pub struct ModuleTree {
    Tree: HashMap<String, Module>,
}

impl ModuleTree {
    pub fn new(path: &Path) -> Self {
        let root = WalkDir::new(path);
        let map: HashMap<_, _> = root
            .into_iter()
            .filter_entry(|e| ModuleProps::check_filename(e))
            .filter_map(|e| e.ok())
            .map(|name| (Module::new(name.path()), name))
            .filter_map(|(x, name)| {
                if let Err(e) = x {
                    eprintln!("Error reading config: {}", e);
                    None
                } else {
                    Some((name.path().to_string_lossy().to_string(), x.unwrap()))
                }
            })
            .collect();
        ModuleTree { Tree: map }
    }
    pub fn run_module<A>(&self, module_name: &str, ip:  A) ->Result<(), Error>
        where A:Display + ToSocketAddrs + Send + Sync + Clone + Debug + Eq + std::hash::Hash + ToString,
    {
        let m_name= module_name.to_string();
        self
            .Tree
            .get(module_name)
            .ok_or(Error::msg(format!("Module {} not found",&module_name)))?
            .execute(ip)
    }
}
