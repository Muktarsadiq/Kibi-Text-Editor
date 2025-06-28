# Kibi Editor

A lightweight, Vim based text editor written in Rust, inspired by the classic "Build Your Own Text Editor" tutorial but reimagined with Rust's safety and performance benefits.

## Features

- **Syntax Highlighting**: Full support for Rust code with keyword, type, string, and comment highlighting
- **Search Functionality**: Interactive search with arrow key navigation and match highlighting  
- **File Operations**: Open, edit, and save files with proper dirty state tracking
- **Terminal Integration**: Raw mode terminal handling with proper cleanup
- **Navigation**: Full cursor movement with arrow keys, Page Up/Down, Home/End
- **Text Editing**: Insert, delete, backspace with proper line joining and splitting
- **Status Bar**: Real-time file information and modification status
- **Multi-line Comments**: Proper handling of `/* */` style comments in Rust files

## Installation

Make sure you have Rust installed, then:

```bash
git clone https://github.com/yourusername/kibi-editor
cd kibi-editor
cargo build --release
```

## Usage

```bash
# Open a file
cargo run -- filename.rs

# Start with empty file
cargo run
```

### Key Bindings

| Key | Action |
|-----|--------|
| `Ctrl+S` | Save file |
| `Ctrl+Q` | Quit (with unsaved changes confirmation) |
| `Ctrl+F` | Search |
| `Arrow Keys` | Navigate |
| `Page Up/Down` | Scroll by screen |
| `Home/End` | Beginning/End of line |
| `Backspace/Delete` | Delete characters |
| `Enter` | New line |
| `ESC` | Cancel search/operations |

### Search Features

- **Incremental Search**: Results update as you type
- **Navigation**: Use arrow keys to jump between matches
- **Highlighting**: Current match is highlighted in blue
- **Wraparound**: Search continues from beginning when reaching end

## Architecture

The editor is built around a central `EditorConfig` struct that manages:

- **Terminal State**: Raw mode handling with proper restoration
- **File Buffer**: Dynamic row management with efficient string operations  
- **Rendering**: Optimized screen updates with escape sequences
- **Syntax Engine**: Extensible highlighting system with keyword detection
- **Search Engine**: Pattern matching with state preservation

### Key Components

- `EditorRow`: Individual line management with rendering and highlighting
- `AppendBuffer`: Efficient screen update batching
- `EditorSyntax`: Language-specific highlighting rules
- `EditorHighlight`: Color coding for different token types

## Dependencies

- `termion`: Terminal size detection and utilities
- `termios`: Low-level terminal control

## Development Journey

This project represents a complete rewrite of the classic C-based text editor tutorial in Rust. Key challenges overcome:

- **Memory Safety**: Leveraging Rust's ownership system for safe buffer management
- **Error Handling**: Proper `Result<T, E>` usage throughout the codebase
- **Unicode Support**: Handling multi-byte characters in terminal rendering
- **Terminal Control**: Raw mode management with guaranteed cleanup
- **Pattern Matching**: Rust's powerful `match` expressions for key handling

## Contributing

Contributions are welcome! Areas for improvement:

- Additional language syntax support
- Configuration file support
- More advanced search features (regex, case sensitivity)
- Split window/tabs functionality
- Plugin system


## Acknowledgments

Inspired by the "Build Your Own Text Editor" tutorial, reimagined for the Rust ecosystem with modern safety and performance considerations.