use anyhow::Error;
use serde::{Deserialize, Deserializer};
use ssh2::Session;
use std::collections::HashMap;
use std::fmt::{Debug, Display};
use std::fs::File;
use std::io::Read;
use std::net::{TcpStream, ToSocketAddrs};
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
            _ => panic!(format!("Bad module type provided: {}", a)),
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

///Shell: Toml of modules
/// like
/// ```toml
/// first = "ls"
/// second = "uptime"
/// ```
/// Binary: Upload binary and execute
/// Python:
/// ```bash
/// python -c python code
/// ```
enum ModuleContent {
    Shell(HashMap<String, String>),
    Binary(PathBuf),
    Python(String),
}

struct Module {
    module_type: ExecType,
    module_content: ModuleContent,
}

impl ModuleProps {
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
pub enum AuthType {
    AgentFirst(String),
    AgentWithKeyName(String, String),
}

impl AuthType {
    pub fn auth(&self, sess: &Session) -> Result<(), Error> {
        match self {
            AuthType::AgentFirst(username) => {
                sess.userauth_agent(&username)?;
            }
            AuthType::AgentWithKeyName(username, key) => unimplemented!(),
        };
        Ok(())
    }
}

pub trait ConnectionProps {
    fn get_timeout(&self) -> u32;
    fn tcp_synchronization(&self);
    fn agent_synchronization(&self);
}

impl Module {
    pub fn new(path: &Path) -> Result<Module, Error> {
        let file_2_string = |p: &Path| -> Result<String, std::io::Error> {
            let mut file = File::open(p)?;
            let mut content = String::new();
            file.read_to_string(&mut content)?;
            Ok(content)
        };

        let res: ModuleProps = from_str(&file_2_string(path)?)?;
        let content = match res.module_type {
            ExecType::Bin => ModuleContent::Binary(res.exec_path),
            ExecType::Python => ModuleContent::Python(file_2_string(&res.exec_path)?),
            ExecType::Bash => {
                let unparsed = file_2_string(&res.exec_path)?;
                let table: HashMap<_, _> = from_str(&unparsed)?;
                ModuleContent::Shell(table)
            }
        };
        Ok(Module {
            module_type: res.module_type,
            module_content: content,
        })
    }

    fn obtain_connection_and_auth<A>(
        &self,
        ip: A,
        auth: AuthType,
        sync: &dyn ConnectionProps,
    ) -> Result<Session, Error>
    where
        A: Display + ToSocketAddrs + Send + Sync + Clone + Debug + Eq + std::hash::Hash + ToString,
    {
        sync.tcp_synchronization();
        let tcp = TcpStream::connect(ip)?;
        let mut sess =
            Session::new().map_err(|_e| Error::msg("Error initializing session".to_string()))?;
        sess.set_tcp_stream(tcp);
        sess.set_timeout(sync.get_timeout());
        sess.handshake()
            .map_err(|e| Error::msg(format!("Failed establishing handshake: {}", e)))?;
        sync.agent_synchronization(); //todo fixme
        auth.auth(&sess)
            .map_err(|e| Error::msg(format!("Authentication Error {}", e)))?;
        Ok(sess)
    }

    fn execute_bash_script<A>(
        &self,
        ip: A,
        auth: AuthType,
        sync: &dyn ConnectionProps,
    ) -> Result<HashMap<String, String>, Error>
    where
        A: Display + ToSocketAddrs + Send + Sync + Clone + Debug + Eq + std::hash::Hash + ToString,
    {
        let session = self.obtain_connection_and_auth(ip, auth, sync)?;
        let content = match &self.module_content {
            ModuleContent::Shell(map) => map,
            _ => unreachable!(),
        };
        let mut res_map = HashMap::new();
        let mut channel = &session.channel_session()?;
        for (command_name, command) in content {
            let mut result_string = String::new();
            channel.exec(&command);
            channel.read_to_string(&mut result_string);
            res_map.insert(command_name.to_string(), result_string);
        }
        Ok(res_map)
    }

    pub fn execute<A>(
        &self,
        ip: A,
        auth: AuthType,
        sync: &dyn ConnectionProps,
    ) -> Result<String, Error>
    where
        A: Display + ToSocketAddrs + Send + Sync + Clone + Debug + Eq + std::hash::Hash + ToString,
    {
        match self.module_type {
            ExecType::Bash => self.execute_bash_script(ip, auth, sync),
            ExecType::Python => (),
            ExecType::Bin => (),
        };
        Ok(String)
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
    // pub fn run_module<A>(&self, module_name: &str, ip:  A, auth: AuthType) ->Result<(), Error>
    //     where A:Display + ToSocketAddrs + Send + Sync + Clone + Debug + Eq + std::hash::Hash + ToString,
    // {
    //     let m_name= module_name.to_string();
    //     self
    //         .Tree
    //         .get(module_name)
    //         .ok_or(Error::msg(format!("Module {} not found",&module_name)))?
    //         .execute(ip, AuthType);
    // }
    // pub fn run_all<A>(&self, ip:  A, auth: AuthType) ->Result<(), Error>
    //     where A:Display + ToSocketAddrs + Send + Sync + Clone + Debug + Eq + std::hash::Hash + ToString,
    // {
    //     unimplemented!();
    // }
}
