use crate::certificate::Certificate;
use crate::config::HookType;
use acme_common::error::Error;
use handlebars::Handlebars;
use log::debug;
use serde::Serialize;
use std::collections::hash_map::Iter;
use std::collections::HashMap;
use std::fs::File;
use std::io::prelude::*;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::{env, fmt};

pub trait HookEnvData {
    fn set_env(&mut self, env: &HashMap<String, String>);
    fn get_env(&self) -> Iter<String, String>;
}

fn deref<F, G>(t: (&F, &G)) -> (F, G)
where
    F: Clone,
    G: Clone,
{
    ((*(t.0)).to_owned(), (*(t.1)).to_owned())
}

macro_rules! imple_hook_data_env {
    ($t: ty) => {
        impl HookEnvData for $t {
            fn set_env(&mut self, env: &HashMap<String, String>) {
                for (key, value) in env::vars().chain(env.iter().map(deref)) {
                    self.env.insert(key, value);
                }
            }

            fn get_env(&self) -> Iter<String, String> {
                self.env.iter()
            }
        }
    };
}

#[derive(Clone, Serialize)]
pub struct PostOperationHookData {
    pub domains: Vec<String>,
    pub algorithm: String,
    pub status: String,
    pub is_success: bool,
    pub env: HashMap<String, String>,
}

imple_hook_data_env!(PostOperationHookData);

#[derive(Clone, Serialize)]
pub struct ChallengeHookData {
    pub domain: String,
    pub challenge: String,
    pub file_name: String,
    pub proof: String,
    pub is_clean_hook: bool,
    pub env: HashMap<String, String>,
}

imple_hook_data_env!(ChallengeHookData);

#[derive(Clone, Serialize)]
pub struct FileStorageHookData {
    // TODO: add the current operation (create/edit)
    pub file_name: String,
    pub file_directory: String,
    pub file_path: PathBuf,
    pub env: HashMap<String, String>,
}

imple_hook_data_env!(FileStorageHookData);

#[derive(Clone, Debug)]
pub struct Hook {
    pub name: String,
    pub hook_type: Vec<HookType>,
    pub cmd: String,
    pub args: Option<Vec<String>>,
    pub stdin: Option<String>,
    pub stdout: Option<String>,
    pub stderr: Option<String>,
    pub allow_failure: bool,
}

impl fmt::Display for Hook {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.name)
    }
}

macro_rules! get_hook_output {
    ($out: expr, $reg: ident, $data: expr) => {{
        match $out {
            Some(path) => {
                let path = $reg.render_template(path, $data)?;
                let file = File::create(path)?;
                Stdio::from(file)
            }
            None => Stdio::null(),
        }
    }};
}

fn call_single<T>(data: &T, hook: &Hook) -> Result<(), Error>
where
    T: Clone + HookEnvData + Serialize,
{
    debug!("Calling hook: {}", hook.name);
    let reg = Handlebars::new();
    let mut v = vec![];
    let args = match &hook.args {
        Some(lst) => {
            for fmt in lst.iter() {
                let s = reg.render_template(fmt, &data)?;
                v.push(s);
            }
            v.as_slice()
        }
        None => &[],
    };
    debug!("Hook {}: cmd: {}", hook.name, hook.cmd);
    debug!("Hook {}: args: {:?}", hook.name, args);
    let mut cmd = Command::new(&hook.cmd)
        .envs(data.get_env())
        .args(args)
        .stdout(get_hook_output!(&hook.stdout, reg, &data))
        .stderr(get_hook_output!(&hook.stderr, reg, &data))
        .stdin(match &hook.stdin {
            Some(_) => Stdio::piped(),
            None => Stdio::null(),
        })
        .spawn()?;
    if hook.stdin.is_some() {
        let data_in = reg.render_template(&hook.stdin.to_owned().unwrap(), &data)?;
        debug!("Hook {}: stdin: {}", hook.name, data_in);
        let stdin = cmd.stdin.as_mut().ok_or("stdin not found")?;
        stdin.write_all(data_in.as_bytes())?;
    }
    // TODO: add a timeout
    let status = cmd.wait()?;
    if !status.success() && !hook.allow_failure {
        let msg = match status.code() {
            Some(code) => format!("Hook {}: unrecoverable failure: code {}", hook.name, code),
            None => format!("Hook {}: unrecoverable failure", hook.name),
        };
        return Err(msg.into());
    }
    match status.code() {
        Some(code) => debug!("Hook {}: exited: code {}", hook.name, code),
        None => debug!("Hook {}: exited", hook.name),
    };
    Ok(())
}

pub fn call<T>(cert: &Certificate, data: &T, hook_type: HookType) -> Result<(), Error>
where
    T: Clone + HookEnvData + Serialize,
{
    for hook in cert
        .hooks
        .iter()
        .filter(|h| h.hook_type.contains(&hook_type))
    {
        call_single(data, &hook)?;
    }
    Ok(())
}
