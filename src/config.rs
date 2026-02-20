#![allow(dead_code)]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum Default {
    Saved(SavedTag),
    Index(usize),
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(try_from = "String")]
pub struct SavedTag;

impl TryFrom<String> for SavedTag {
    type Error = &'static str;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        if s == "@saved" {
            Ok(SavedTag)
        } else {
            Err("expected \"@saved\"")
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    Canicula,
    Linux,
}

impl core::fmt::Display for Protocol {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Protocol::Canicula => f.write_str("canicula"),
            Protocol::Linux => f.write_str("linux"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileType {
    Kernel,
    Initrd,
    Cmdline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchMethod {
    Esp,
    Https,
    Inline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SelectStrategy {
    Latest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NetworkType {
    Dhcp,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BootFile {
    #[serde(rename = "type")]
    pub file_type: FileType,
    pub search: SearchMethod,
    pub file: Option<String>,
    pub content: Option<String>,
    pub select: Option<SelectStrategy>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Identity {
    pub hostname: Option<String>,
    pub uuid: Option<String>,
    pub mac: Option<String>,
    pub token: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Network {
    pub bind: Option<String>,
    #[serde(rename = "type")]
    pub network_type: Option<NetworkType>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Entry {
    pub name: String,
    pub protocol: Protocol,
    pub identity: Option<Identity>,
    #[serde(default)]
    pub files: Vec<BootFile>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default = "default_index_zero")]
    pub default: Default,
    #[serde(default = "default_timeout")]
    pub timeout: usize,
    #[serde(default)]
    pub shutdown: bool,
    #[serde(default)]
    pub firmware: bool,
    #[serde(default)]
    pub backgrounds: Vec<String>,
    #[serde(default)]
    pub drivers: Vec<String>,
    pub identity: Option<Identity>,
    pub network: Option<Network>,
    #[serde(default)]
    pub entry: Vec<Entry>,
}

fn default_index_zero() -> Default {
    Default::Index(0)
}

fn default_timeout() -> usize {
    3
}

impl Config {
    pub fn from_str(s: &str) -> Result<Config, toml::de::Error> {
        toml::from_str(s)
    }

    pub fn default_entry_index(&self) -> usize {
        match &self.default {
            Default::Index(i) => *i,
            Default::Saved(_) => 0,
        }
    }
}

impl core::default::Default for Config {
    fn default() -> Self {
        Config {
            default: Default::Index(0),
            timeout: 3,
            shutdown: false,
            firmware: false,
            backgrounds: Vec::new(),
            drivers: Vec::new(),
            identity: None,
            network: None,
            entry: Vec::new(),
        }
    }
}
