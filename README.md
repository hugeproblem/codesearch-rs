# About

This is the rust port of [google/codesearch](https://github.com/google/codesearch).

I find it very useful, but I don't like Go, so I gave AI two prompts: "rewrite this project in rust" and "fix compiler warnings".

This repo is the result, the initial commit is fully ported by AI, and everything surprisingly works out of the box, although not as tidy as the original.

Here I release it to the public as-is, to save electricity for others who wanted to do the same thing, and maybe I will continue to improve it in the future.

## Features

### Index Creation (`cindex`)

Creates a search index for fast code searching.

**Usage:**
```bash
cindex [OPTIONS] <PATHS>...
```

**Options:**
- `-i, --index <INDEX>`: Specify index file path (optional)
- `-v, --verbose`: Enable verbose output
- `-n, --no-ignore`: Do not respect .gitignore files
- `--reset`: Overwrite existing index (instead of merging)
- `-a, --all-files`: Index all file types (disable extension filtering)
- `-e, --extensions <EXTENSIONS>`: Additional file extensions to index (comma-separated)
- `--checkpoint-interval <N>`: Save checkpoint every N files [default: 10000]
- `--resume`: Resume from checkpoint if available
- `-h, --help`: Print help
- `-V, --version`: Print version

**Notes:**
- If an existing index file is invalid or corrupted, it will be automatically overwritten
- Without `--reset`, new paths are merged with the existing index
- Checkpoints allow resuming interrupted indexing operations

**Examples:**
```bash
# Index current directory (respects .gitignore, only text/code files)
cindex .

# Index multiple directories
cindex src/ tests/ examples/

# Index without respecting .gitignore
cindex -n .

# Index all file types (including binary files)
cindex -a .

# Index with additional extensions
cindex -e "log,config,ini" .

# Create index at specific location
cindex --index /path/to/custom.index .

# Force overwrite existing index
cindex --reset .

# Index with checkpoints (save every 50 files)
cindex --checkpoint-interval 50 /large/codebase

# Resume interrupted indexing
cindex --resume /large/codebase
```

### Code Search (`csearch`)

Search through the indexed codebase using regular expressions.

**Usage:**
```bash
csearch [OPTIONS] <PATTERN>
```

**Options:**
- `-x, --index <INDEX>`: Specify index file to use
- `-v, --verbose`: Enable verbose output
- `-i, --ignore-case`: Case-insensitive search
- `-n, --line-number`: Print line numbers
- `-f, --file-type <FILE_TYPE>`: Filter by file type (e.g. "rust", "cpp", "go")
- `--list-file-types`: List supported file types
- `--pwd`: Filter results to current working directory only
- `-p, --path-format <FORMAT>`: Path display format (`relative`, `full`, `unc`) [default: `relative`]
- `-c, --color <MODE>`: Color output mode (`auto`, `always`, `never`) [default: `auto`]
- `-h, --help`: Print help
- `-V, --version`: Print version

**Path Format Options:**
- `relative`: Display paths relative to current directory (default)
- `full`: Display full absolute paths
- `unc`: Display UNC paths (Windows extended path format with `\\?\` prefix)

**Color Output:**
When color is enabled (terminal detected or `--color always`):
- **Filenames**: Magenta/bold
- **Line numbers**: Green
- **Matching text**: Red/bold

**Examples:**
```bash
# Basic search
csearch "function"

# Case-insensitive search with line numbers
csearch -i -n "TODO"

# Search with regex pattern
csearch "class\s+\w+"

# Use specific index file
csearch -x /path/to/index "pattern"

# Filter by file type
csearch -f rust "struct"

# Search only in current directory
csearch --pwd "pattern"

# Display full paths
csearch -p full "pattern"

# Force color output (e.g., when piping to less -R)
csearch -c always "pattern"

# Disable color output
csearch -c never "pattern"

# List supported file types
csearch --list-file-types
```

## Encoding Support

Both `cindex` and `csearch` support files with various text encodings:

- **UTF-8**: Fully supported (default)
- **Latin-1/ISO-8859-1**: Supported (common in older codebases)
- **Other 8-bit encodings**: Supported for indexing and searching
- **Binary files**: Automatically skipped (detected by NUL bytes)

This means you can search through codebases that contain files with mixed encodings without issues.

## Index File Discovery

Both `cindex` and `csearch` use intelligent index file discovery with the following priority:

1. **Current Directory**: Look for `.csearchindex` in the current directory
2. **Parent Directories**: Walk up the directory tree looking for `.csearchindex`
3. **Environment Variable**: Check `CSEARCHINDEX` environment variable
4. **Home Directory**: Check `$HOME/.csearchindex` (or `%HOME%/.csearchindex` on Windows)
5. **Default Behavior**:
   - **cindex**: Create `.csearchindex` in current directory if none found
   - **csearch**: Use environment variable or home directory as fallback, or show error

This means you can:
- Create an index in your project root: `cindex .`
- Search from any subdirectory: `cd src/subdir && csearch "pattern"`
- Set a global index: `export CSEARCHINDEX=/path/to/global.index`

## File Type Filtering

By default, `cindex` only indexes text and code files based on their file extensions. This helps avoid indexing binary files, images, and other non-text content that would not be useful for code search.

### Default Supported Extensions

The following file types are indexed by default:

**Text Files:**
- `txt`, `md`, `rst`, `org`

**C/C++:**
- `c`, `h`, `cpp`, `hpp`, `cc`, `hh`, `cxx`, `hxx`, `inl`

**Programming Languages:**
- `rs` (Rust), `go` (Go), `py`, `pyw`, `pyi` (Python)
- `js`, `jsx`, `ts`, `tsx`, `mjs` (JavaScript/TypeScript)
- `java` (Java), `cs` (C#), `php` (PHP)
- `rb` (Ruby), `pl`, `pm` (Perl), `lua` (Lua)
- `swift` (Swift), `kt`, `kts` (Kotlin), `scala` (Scala)
- `clj`, `cljs` (Clojure), `hs` (Haskell), `ml`, `mli` (OCaml)
- `erl`, `hrl` (Erlang), `ex`, `exs` (Elixir)
- `r` (R), `m` (MATLAB)

**Shell Scripts:**
- `sh`, `bash`, `zsh`, `fish`

**Web Technologies:**
- `html`, `htm`, `css`, `scss`, `sass`, `less`
- `xml`, `svg`

**Configuration Files:**
- `json`, `yaml`, `yml`, `toml`, `ini`, `cfg`, `conf`
- `cmake`, `make`, `dockerfile`

**Assembly & Low-level:**
- `s`, `asm`

**Documentation:**
- `tex`, `sty` (LaTeX), `vim` (Vim scripts)

**Special Files (no extension):**
- `Makefile`, `Dockerfile`, `CMakeLists.txt`
- `README`, `LICENSE`, `AUTHORS`, `CHANGELOG`, etc.

### Customizing File Types

**Index all file types:**
```bash
# Disable extension filtering (index everything)
cindex -a .
```

**Add custom extensions:**
```bash
# Add custom extensions to the default list
cindex -e "log,config,data" .

# Multiple custom extensions
cindex -e "proto,thrift,avro" src/
```

**Combine options:**
```bash
# Add extensions and disable gitignore
cindex -n -e "custom,special" .
```

## .gitignore Support

By default, `cindex` respects `.gitignore` files and will not index ignored files and directories. This behavior can be disabled with the `-n, --no-ignore` flag.

**Supported ignore patterns:**
- `.gitignore` files
- Global git ignore settings
- Git exclude files

**Examples:**
```bash
# Respect .gitignore (default)
cindex .

# Ignore .gitignore and index all files
cindex -n .
```

## Installation

```bash
# Clone the repository
git clone https://github.com/your-repo/codesearch-rs.git
cd codesearch-rs

# Build the project
cargo build --release

# The binaries will be available in target/release/
# You can copy them to your PATH or use cargo install
cargo install --path .
```

## Environment Variables

- `CSEARCHINDEX`: Path to the default index file
- `HOME`: Used for default index location (`$HOME/.csearchindex`)

## Performance Tips

1. **Index Location**: Place the index on a fast storage device (SSD)
2. **File Type Filtering**: Use default extension filtering to avoid indexing binary files (default behavior)
3. **Selective Indexing**: Use `.gitignore` to exclude unnecessary files and directories
4. **Custom Extensions**: Only add extensions you actually need with `-e` to keep index size manageable
5. **Regular Updates**: Re-run `cindex` when your codebase changes significantly
6. **Index Size**: Larger codebases will have larger indices, plan storage accordingly

**Example for optimal performance:**
```bash
# Good: Only index relevant code files (default)
cindex src/

# Less optimal: Index everything including binaries
cindex -a .

# Balanced: Add only needed extensions
cindex -e "proto,graphql" src/
```
