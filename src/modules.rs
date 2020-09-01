use anyhow::Error;
use base64::encode;
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

#[derive(Debug,Clone)]
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

#[derive(Debug,Clone)]enum ModuleContent {
    Shell(HashMap<String, String>),
    Binary(PathBuf),
    Python(String),
}

pub enum CommandOutput {
    Multi(HashMap<String, String>),
    Single(String),
}

#[derive(Debug,Clone)]
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
        ext == "mod"
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
    fn tcp_release(&self);
    fn agent_release(&self);
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
            ExecType::Python => {
                let content = file_2_string(&res.exec_path)?;
                let com64 = encode(content);
                let script = format!("python2 -c \" exec('{}'.decode('base64'))\"", com64);
                ModuleContent::Python(script)
            }
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
        sync.agent_release();
        Ok(sess)
    }

    fn execute_python_script<A>(
        &self,
        ip: A,
        auth: AuthType,
        sync: &dyn ConnectionProps,
    ) -> Result<String, Error>
    where
        A: Display + ToSocketAddrs + Send + Sync + Clone + Debug + Eq + std::hash::Hash + ToString,
    {
        let session = self.obtain_connection_and_auth(ip, auth, sync)?;
        let content = match &self.module_content {
            ModuleContent::Python(script) => script,
            _ => unreachable!(),
        };
        let mut channel = session.channel_session()?;
        channel.exec(&content)?;
        let mut result = String::new();
        channel.read_to_string(&mut result)?;
        Ok(result)
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
        let mut channel = session.channel_session()?;
        for (command_name, command) in content {
            let mut result_string = String::new();
            channel.exec(&command)?;
            channel.read_to_string(&mut result_string)?;
            res_map.insert(command_name.to_string(), result_string);
        }
        Ok(res_map)
    }

    pub fn execute<A>(
        &self,
        ip: A,
        auth: AuthType,
        sync: &dyn ConnectionProps,
    ) -> Result<CommandOutput, Error>
    where
        A: Display + ToSocketAddrs + Send + Sync + Clone + Debug + Eq + std::hash::Hash + ToString,
    {
        let result =match self.module_type {
            ExecType::Bash => self
                .execute_bash_script(ip, auth, sync)
                .map(CommandOutput::Multi),
            ExecType::Python => unimplemented!(),
            ExecType::Bin => unimplemented!(),
        };
        sync.tcp_release();
        result
    }
}
#[derive(Debug,Clone)]
pub struct ModuleTree {
    tree: HashMap<String, Module>,
}

impl ModuleTree {
    pub fn check_module(&self, module_name: &str) -> bool
    {
        self.tree.contains_key(module_name)

    }
    pub fn new(path: &Path) -> Self {
        let root = WalkDir::new(path).max_depth(1);
        let map: HashMap<_, _> = root
            .into_iter()
            .filter_map(|e| e.ok()) //filter erros
            .filter(|f| ModuleProps::check_filename(f)) //leave only mods
            .map(|name| (Module::new(name.path()), name)) //try to create module
            .filter_map(|(x, name)| {
                if let Err(e) = x {
                    eprintln!("Error parsing module {}: {}",name.file_name().to_string_lossy(), e);
                    None
                } else {
                    Some((
                        name.path()
                            .file_name()
                            .expect("Failed getting filename for module, which is strange")
                            .to_string_lossy()
                            .to_string(),
                        x,
                    ))
                }
            })
            .filter_map(|(name, module)| {
                if let Err(e) = &module {
                    eprintln!("{}", e);
                }
                module.map(|x| (name, x)).ok()
            })
            .collect();

        ModuleTree { tree: map }
    }
    pub fn run_module<A>(
        &self,
        module_name: &str,
        ip: A,
        auth: AuthType,
        sync: &dyn ConnectionProps,
    ) -> Result<CommandOutput, Error>
    where
        A: Display + ToSocketAddrs + Send + Sync + Clone + Debug + Eq + std::hash::Hash + ToString,
    {
        self.tree
            .get(module_name)
            .ok_or_else(|| Error::msg(format!("Module {} not found", &module_name)))?
            .execute(ip, auth, sync)
    }
    pub fn run_all<A>(&self, ip: A, auth: AuthType, sync: &dyn ConnectionProps) -> Result<(), Error>
    where
        A: Display + ToSocketAddrs + Send + Sync + Clone + Debug + Eq + std::hash::Hash + ToString,
    {
        unimplemented!();
    }
}

