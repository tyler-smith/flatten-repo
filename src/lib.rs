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

pub struct Config {
    pub recursive: bool,
    pub verbose: bool,
    pub exclude_patterns: Vec<String>,
    pub paths: Vec<String>,
}

pub struct FlattenRepo {
    config: Config,
    git_repo: Option<Repository>,
    exclude_patterns: Vec<Pattern>,
}

impl FlattenRepo {
    pub fn new(config: Config) -> Result<Self> {
        let git_repo = Repository::discover(".").ok();

        let _ = Pattern::parse(without_escape("*.git"))?;

        // Parse the exclude patterns once during initialization
        let exclude_patterns = config.exclude_patterns
            .iter()
            .map(|p| Pattern::parse(without_escape(p)).map_err(Error::from))
            .collect::<Result<Vec<_>>>()?;

        Ok(Self { config, git_repo, exclude_patterns })
    }

    fn log_verbose(&self, msg: &str) {
        if self.config.verbose {
            eprintln!("{}", msg);
        }
    }

    fn is_ignored_by_git(&self, path: &Path) -> bool {
        if let Some(repo) = &self.git_repo {
            repo.status_should_ignore(path).unwrap_or(false)
        } else {
            false
        }
    }

    fn should_exclude(&self, path: &str) -> bool {
        self.exclude_patterns.iter().any(|pattern| {
            // Find any match in the string
            pattern.find(path).is_some()
        })
    }

    fn is_binary_file(path: &Path) -> Result<bool> {
        let mut file = File::open(path)?;
        let mut buffer = [0; 8192];
        let n = file.read(&mut buffer)?;
        Ok(buffer[..n].contains(&0))
    }

    fn read_file_contents(path: &Path) -> Result<(bool, Option<String>)> {
        if Self::is_binary_file(path)? {
            return Ok((true, None));
        }

        let file = File::open(path)?;
        let mut decoder = DecodeReaderBytesBuilder::new()
            .encoding(Some(UTF_8))
            .build(file);

        let mut content = String::new();
        decoder.read_to_string(&mut content)?;
        Ok((false, Some(content)))
    }

    fn find_files(&self) -> Result<Vec<PathBuf>> {
        let mut seen_files = std::collections::HashSet::new();
        let mut files = Vec::new();

        for pattern in &self.config.paths {
            self.log_verbose(&format!("Processing pattern: {}", pattern));

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
                    if self.should_exclude(relpath.to_string_lossy().as_ref()) {
                        self.log_verbose(&format!("Excluding file (matched exclude pattern): {}", relpath.display()));
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

                        if self.should_exclude(relpath.to_string_lossy().as_ref()) {
                            self.log_verbose(&format!("Excluding file (matched exclude pattern): {}", relpath.display()));
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

    pub fn generate_xml(&self) -> Result<String> {
        let files = self.find_files()?;
        let mut writer = Writer::new(Vec::new());

        // Write XML declaration
        writer.write_event(Event::Decl(BytesDecl::new("1.0", Some("UTF-8"), None)))?;


        // Write root element
        writer.write_event(Event::Start(BytesStart::new("repo")))?;

        for path in files {
            let (is_binary, content) = Self::read_file_contents(&path)?;
            let path_str = path.to_string_lossy();

            let mut elem = BytesStart::new("file");
            elem.push_attribute(("path", path_str.as_ref()));

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
                writer.write_event(Event::End(BytesEnd::new("file")))?;
            }
        }

        writer.write_event(Event::End(BytesEnd::new("repo")))?;

        String::from_utf8(writer.into_inner())
            .map_err(|e| Error::Path(e.to_string()))
    }
}