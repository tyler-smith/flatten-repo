use std::{
    fs::File,
    io::{self, Read},
    path::{Path, PathBuf},
};
use encoding_rs::UTF_8;
use encoding_rs_io::DecodeReaderBytesBuilder;
use yash_fnmatch::{Pattern, without_escape};
use git2::Repository;
use quick_xml::{
    events::{BytesEnd, BytesStart, BytesText, Event, BytesDecl},
    Writer,
};
use thiserror::Error;

const XML_NODE_TYPE_REPO: &str = "repository";
const XML_NODE_TYPE_FILE: &str = "file";

const BINARY_FILE_CHECK_SIZE: usize = 8192;

#[derive(Error, Debug)]
pub enum Error {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    #[error("Git error: {0}")]
    Git(#[from] git2::Error),
    #[error("XML error: {0}")]
    Xml(#[from] quick_xml::Error),
    #[error("Pattern error: {0}")]
    GlobPattern(#[from] glob::PatternError),
    #[error("fnmatch error: {0}")]
    FNMatchPattern(#[from] yash_fnmatch::Error),
    #[error("Path error: {0}")]
    Path(String),
}

pub type Result<T> = std::result::Result<T, Error>;

pub enum FileType {
    Text(String),
    Binary,
}

pub struct Config {
    pub recursive: bool,
    pub verbose: bool,
    pub ignore_patterns: Vec<String>,
    pub paths: Vec<String>,
}

pub struct FlattenRepo {
    config: Config,
    git_repo: Option<Repository>,
    ignore_patterns: Vec<Pattern>,
}

impl FlattenRepo {
    pub fn new(config: Config) -> Result<Self> {
        // If no paths are provided, default to the current directory
        let config = if config.paths.is_empty() {
            let mut config = config;
            config.paths = vec![".".to_string()];
            config
        } else {
            config
        };

        // Find the git repository if we're in one
        let git_repo = Repository::discover(".").ok();

        // Parse the ignore patterns once during initialization
        let ignore_patterns = config.ignore_patterns
            .iter()
            .map(|p| Pattern::parse(without_escape(p)).map_err(Error::from))
            .collect::<Result<Vec<_>>>()?;

        Ok(Self { config, git_repo, ignore_patterns })
    }

    // Generate the XML structure from the files found based on the config.
    pub fn generate_xml(&self) -> Result<String> {
        let files = self.find_files()?;
        self.generate_xml_from_files(files)
    }

    // Logs messages if and only if we're in verbose mode.
    fn log_verbose(&self, msg: &str) {
        if self.config.verbose {
            eprintln!("{}", msg);
        }
    }

    // Check if a path is ignored by git
    fn is_ignored_by_git(&self, path: &Path) -> bool {
        self.git_repo.as_ref().map_or(false, |repo| {
            repo.status_should_ignore(path).unwrap_or(false)
        })
    }

    // Check if a path is ignored by any of our ignore patterns
    fn is_ignored_by_patterns(&self, path: &str) -> bool {
        self.ignore_patterns.iter().any(|pattern| {
            pattern.find(path).is_some()
        })
    }

    // Find all files matching the patterns in the config, respecting ignore conditions.
    fn find_files(&self) -> Result<Vec<PathBuf>> {
        let mut seen_files = std::collections::HashSet::new();
        let mut files = Vec::new();

        for pattern in &self.config.paths {
            self.log_verbose(&format!("Processing pattern: {}", pattern));

            // Append globstar to directories if we want to recurse
            let pattern = if self.config.recursive && Path::new(pattern).is_dir() {
                format!("{}/**/*", pattern)
            } else {
                pattern.clone()
            };

            // Handle direct file paths
            if Path::new(&pattern).is_file() {
                let path = PathBuf::from(&pattern);
                let relpath = path.strip_prefix(".").unwrap_or(&path);
                if !seen_files.contains(relpath) {
                    if self.is_ignored_by_patterns(relpath.to_string_lossy().as_ref()) {
                        self.log_verbose(&format!("Excluding file (matched ignore pattern): {}", relpath.display()));
                        continue;
                    }
                    if self.is_ignored_by_git(relpath) {
                        self.log_verbose(&format!("Excluding file (matched gitignore): {}", relpath.display()));
                        continue;
                    }
                    self.log_verbose(&format!("Including file: {}", relpath.display()));
                    seen_files.insert(relpath.to_path_buf());
                    files.push(relpath.to_path_buf());
                }
                continue;
            }

            // Process glob patterns
            for entry in glob::glob(&pattern)? {
                match entry {
                    Ok(path) => {
                        if !path.is_file() {
                            self.log_verbose(&format!("Skipping non-file: {}", path.display()));
                            continue;
                        }

                        let relpath = path.strip_prefix(".").unwrap_or(&path);
                        if seen_files.contains(relpath) {
                            self.log_verbose(&format!("Skipping duplicate file: {}", relpath.display()));
                            continue;
                        }

                        if self.is_ignored_by_patterns(relpath.to_string_lossy().as_ref()) {
                            self.log_verbose(&format!("Excluding file (matched ignore pattern): {}", relpath.display()));
                            continue;
                        }

                        if self.is_ignored_by_git(relpath) {
                            self.log_verbose(&format!("Excluding file (matched gitignore): {}", relpath.display()));
                            continue;
                        }

                        self.log_verbose(&format!("Including file: {}", relpath.display()));
                        seen_files.insert(relpath.to_path_buf());
                        files.push(relpath.to_path_buf());
                    }
                    Err(e) => self.log_verbose(&format!("Error processing glob: {}", e)),
                }
            }
        }

        Ok(files)
    }

    // Read the contents of a file, checking for binary content.
    // If the file is binary, we stop reading and don't return the contents.
    fn read_file_contents(path: &Path) -> Result<(bool, Option<String>)> {
        let file = File::open(path)?;
        let mut decoder = DecodeReaderBytesBuilder::new()
            .encoding(Some(UTF_8))
            .build(file);

        // Read initial chunk and return early if the content appears to be binary
        let mut initial_buffer = Vec::with_capacity(BINARY_FILE_CHECK_SIZE);
        {
            let mut limited_reader = decoder.by_ref().take(BINARY_FILE_CHECK_SIZE as u64);
            let bytes_read = limited_reader.read_to_end(&mut initial_buffer)?;
            if initial_buffer[..bytes_read].contains(&0) {
                return Ok((true, None));
            }
        }

        // We have a text file so convert initial chunk to string and read the rest
        let mut content = String::with_capacity(initial_buffer.len());
        content.push_str(&String::from_utf8_lossy(&initial_buffer));
        decoder.read_to_string(&mut content)?;

        Ok((false, Some(content)))
    }

    // Create the XML structure from the given file list.
    fn generate_xml_from_files(&self, files: Vec<PathBuf>) -> Result<String> {
        // Create an XML writer and start writing the document
        let mut writer = Writer::new(Vec::new());
        writer.write_event(Event::Decl(BytesDecl::new("1.0", Some("UTF-8"), None)))?;
        writer.write_event(Event::Start(BytesStart::new(XML_NODE_TYPE_REPO)))?;

        // Iterate files and create elements for each one
        for path in files {
            // Create the file element
            let mut elem = BytesStart::new(XML_NODE_TYPE_FILE);
            let path_str = path.to_string_lossy();
            elem.push_attribute(("path", path_str.as_ref()));

            // Add the file contents/attributes
            let (is_binary, content) = Self::read_file_contents(&path)?;
            if is_binary {
                self.log_verbose(&format!("Processing as binary file: {}", path_str));
                elem.push_attribute(("binary", "true"));
                writer.write_event(Event::Empty(elem))?;
            } else {
                self.log_verbose(&format!("Processing as text file: {}", path_str));
                writer.write_event(Event::Start(elem))?;
                if let Some(content) = content {
                    writer.write_event(Event::Text(BytesText::new(&content)))?;
                }
                writer.write_event(Event::End(BytesEnd::new(XML_NODE_TYPE_FILE)))?;
            }
        }

        // Close the root element
        writer.write_event(Event::End(BytesEnd::new(XML_NODE_TYPE_REPO)))?;

        // Convert to an XML string
        String::from_utf8(writer.into_inner())
            .map_err(|e| Error::Path(e.to_string()))
    }
}