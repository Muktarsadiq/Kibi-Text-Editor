use std::fs::File;
use std::fs::OpenOptions;
use std::io::{self, BufRead, BufReader};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::io::AsRawFd;
use std::path::Path;
use std::time::{Duration, SystemTime};
use termion::terminal_size;
use termios::{
    tcgetattr, tcsetattr, Termios, BRKINT, CS8, ECHO, ICANON, ICRNL, IEXTEN, INPCK, ISIG, ISTRIP,
    IXON, OPOST, TCSAFLUSH, VMIN, VTIME,
};

const VERSION: &str = "0.0.1";
const TAB_STOP: usize = 8; // Number of spaces for a tab stop
const QUIT_TIMES: u8 = 3; // Number of times to press Ctrl-Q to quit

// Helper function to convert to ctrl key value - kept outside for simplicity
fn ctrl_key(k: u8) -> u8 {
    k & 0x1f  
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EditorKey {
    ArrowUp,
    ArrowDown,
    ArrowRight,
    ArrowLeft,
    HomeKey,
    EndKey,
    Delete,
    PageUp,
    PageDown,
    CtrlQ,
    Escape,
    EnterKey,
    Backspace,
    CtrlF,
    CtrlH,
    CtrlL,
    CtrlS,
    Other(u8),
}

#[derive(Copy, Clone)]
#[repr(u8)]
pub enum EditorHighlight {
    Normal = 0,
    Number = 1,
    Match,
    HlString,
    HlComment,
    HlMComment,
    HlKeyword1,
    HlKeyword2,
}

// Now no casting needed — Rust auto-converts via `as u8` safely & clearly.

// Error handling function
fn die(message: &str) -> ! {
    let mut stdout = io::stdout();
    // Try to clear the screen before showing error
    let _ = stdout.write_all(b"\x1b[2J\x1b[H");
    let _ = stdout.flush();

    eprintln!("Error: {} {}", message, io::Error::last_os_error());
    std::process::exit(1);
}

//define the buffer structure
struct AppendBuffer {
    buffer: Vec<u8>, // stores the data we want to write to the screen
}

//create a new buffer
impl AppendBuffer {
    fn new() -> Self {
        Self { buffer: Vec::new() }
    }

    //append data to the buffer
    fn append(&mut self, data: &[u8]) {
        self.buffer.extend_from_slice(data);
    }

    fn write_all(&self) -> io::Result<()> {
        let mut stdout = io::stdout();
        stdout.write_all(&self.buffer)?;
        stdout.flush()
    }

    pub fn append_char(&mut self, ch: char) {
        let mut buf = [0; 4];
        let s = ch.encode_utf8(&mut buf);
        self.append(s.as_bytes());
    }
}

const HL_HIGHLIGHT_NUMBERS: usize = 1 << 0;
const HL_HIGHLIGHT_STRINGS: usize = 1 << 1;

pub struct EditorSyntax {
    filetype: &'static str,
    filematch: &'static [&'static str],
    keywords: &'static [&'static str],
    types: &'static [&'static str],
    single_line_comment_start: &'static str,
    multiline_comment_start: &'static str,
    multiline_comment_end: &'static str,
    flags: usize,
}

#[derive(Debug)]
pub struct EditorRow {
    pub size: usize,
    pub chars: String,
    pub render: String,
    pub r_size: usize,
    pub hl: Option<Vec<u8>>,
    idx: usize,
    hl_open_comment: bool
}
impl EditorRow {
    pub fn update_row(&mut self) {
        let mut render = String::new();
        let mut idx = 0;

        for ch in self.chars.chars() {
            if ch == '\t' {
                render.push(' ');
                idx += 1;
                while idx % TAB_STOP != 0 {
                    render.push(' ');
                    idx += 1;
                }
            } else {
                render.push(ch);
                idx += 1;
            }
        }

        self.r_size = render.len();
        self.render = render;
    }

    pub fn insert_char(&mut self, at: usize, c: char) {
        let at = at.min(self.chars.len());
        self.chars.insert(at, c);
        self.size += 1;
        self.update_row();
        
    }

    pub fn delete_char(&mut self, at: usize) {
        if at >= self.chars.len() {
            return;
        }
        self.chars.remove(at);
        self.update_row();
       
    }

    pub fn append_string(&mut self, s: &str) {
        self.chars.push_str(s);
        self.size = self.chars.len();
        self.update_row();
        
    }

    pub fn highlight_match(&mut self, match_index: usize, query: &str) {
        if let Some(ref mut hl) = self.hl {
            let start = match_index;
            let end = start + query.len();

            if end <= hl.len() {
                for i in start..end {
                    hl[i] = EditorHighlight::Match as u8;
                }
            }
        }
    }

    pub fn editor_syntax_to_color(hl: EditorHighlight) -> u8 {
        match hl {
            EditorHighlight::Number => 31,
            EditorHighlight::Match => 34,
            EditorHighlight::HlString => 35,
            EditorHighlight::HlComment | EditorHighlight::HlMComment => 36,
            EditorHighlight::HlKeyword1 => 33,
            EditorHighlight::HlKeyword2 => 32,
            _ => 37,
        }
    }
}

const RUST_EXTENSION: &[&str] = &[".rs", ".toml"];

const RUST_HL_KEYWORDS: &[&str] = &[
    // Control flow keywords (HL_KEYWORD1 - Yellow)
    "if", "else", "while", "for", "loop", "break", "continue", "return",
    "match", 
    
    // Declaration keywords (HL_KEYWORD2 - Green, marked with |)
    "struct|", "enum|", "impl|", "trait|", "fn|", "let|", "mut|",
    "const|", "static|", "pub|", "mod|", "use|", "crate|", "super|", "self|",
];
    
const RUST_TYPES: &[&str] = &[ 
	 "i8", "i16", "i32", "i64", "i128", "isize",
    "u8", "u16", "u32", "u64", "u128", "usize",
    "f32", "f64", "bool", "char", "str", "String",
    "Vec", "Option", "Result",
];

const HLDB: &[EditorSyntax] = &[EditorSyntax {
    filetype: "Rust",
    filematch: RUST_EXTENSION,
    types: RUST_TYPES,
    keywords: RUST_HL_KEYWORDS,
    single_line_comment_start: "//",
    multiline_comment_start: "/*",
    multiline_comment_end: "*/",
    flags: HL_HIGHLIGHT_NUMBERS | HL_HIGHLIGHT_STRINGS,
}];

const HLDB_ENTRIES: usize = HLDB.len();

// Main editor state structure
struct EditorConfig {
    original_termios: Option<Termios>,
    cx: usize,
    cy: usize,
    rx: usize,
    row_off: usize,
    col_off: usize,
    screen_rows: usize,
    screen_cols: usize,
    dirty: usize,
    number_of_rows: usize,
    quit_times: u8,
    erow: Vec<EditorRow>,
    filename: Option<String>,
    status_msg: String,
    status_msg_time: SystemTime,
    saved_hl: Option<Vec<u8>>,
    saved_hl_line: Option<usize>,
    syntax: Option<&'static EditorSyntax>,
}

impl EditorConfig {
    fn new() -> Result<Self, io::Error> {
        //Try to get terminal size but fallback to 80x24
        let (cols, rows) = Self::get_window_size()?;

        Ok(EditorConfig {
            original_termios: None,
            screen_rows: rows.saturating_sub(2), // Leave space for status bar
            screen_cols: cols,
            cx: 0,
            cy: 0,
            rx: 0,
            row_off: 0,
            col_off: 0,
            dirty: 0,
            number_of_rows: 0,
            quit_times: QUIT_TIMES,
            erow: Vec::new(),
            filename: None,
            status_msg: String::new(),
            status_msg_time: SystemTime::now(),
            saved_hl: None,
            saved_hl_line: None,
            syntax: None,
        })
    }

    //get window size
    fn get_window_size() -> io::Result<(usize, usize)> {
        let (width, height) = terminal_size()?;
        Ok((width as usize, height as usize))
    }

    fn highlight_to_color(&self, hl: u8) -> u8 {
        match hl {
        x if x == EditorHighlight::Number as u8 => 31,     // Red
        x if x == EditorHighlight::Match as u8 => 34,      // Blue  
        x if x == EditorHighlight::HlString as u8 => 35,   // Magenta
        x if x == EditorHighlight::HlComment as u8 => 36,  // Cyan
        x if x == EditorHighlight::HlMComment as u8 => 36, // Cyan
        x if x == EditorHighlight::HlKeyword1 as u8 => 33, // Yellow (control flow)
        x if x == EditorHighlight::HlKeyword2 as u8 => 32, // Green (declarations)
        _ => 37, // White (normal)
    }
    }

    fn restore_highlight(&mut self) {
        // If we have saved highlights, restore them
        if let (Some(saved_hl), Some(saved_line)) = (&self.saved_hl, self.saved_hl_line) {
            // Ensure the saved line is still valid
            if saved_line < self.erow.len() {
                // Restore the original highlight
                if let Some(ref mut current_hl) = self.erow[saved_line].hl {
                    // Only restore if the sizes match (safety check)
                    if current_hl.len() == saved_hl.len() {
                        current_hl.copy_from_slice(saved_hl);
                    }
                }
            }

            // Clear the saved state
            self.saved_hl = None;
            self.saved_hl_line = None;
        }
    }

    fn save_highlight(&mut self, line_index: usize) {
        // First restore any existing saved highlights
        self.restore_highlight();

        // Save the current line's highlights
        if line_index < self.erow.len() {
            if let Some(ref hl) = self.erow[line_index].hl {
                // Clone the current highlight vector
                self.saved_hl = Some(hl.clone());
                self.saved_hl_line = Some(line_index);
            }
        }
    }

    fn is_separator(c: char) -> bool {
        c.is_whitespace() || c == '\0' || ",.()+-/*=~%<>[];".contains(c)
    }

    fn editor_row_cx_to_rx(&self, row: &EditorRow, cx: usize) -> usize {
        //initialise rx
        let mut rx = 0;
        //loop through the chars
        for (j, ch) in row.chars.chars().enumerate() {
            if j >= cx {
                break;
            }
            if ch == '\t' {
                // calculate padding to the next tab stop
                rx += (TAB_STOP - 1) - (rx % TAB_STOP);
                //move to the next position
                rx += 1;
            } else {
                rx += 1;
            }
        }
        rx
    }

    fn editor_row_rx_to_cx(&self, row: &EditorRow, rx: usize) -> usize {
        //variable to keep track of rendered index
        let mut cur_rx = 0;

        for (cx, ch) in row.chars.chars().enumerate() {
            if ch == '\t' {
                cur_rx += (TAB_STOP - 1) - (cur_rx % TAB_STOP);
            }
            cur_rx += 1;

            if cur_rx >= rx {
                return cx;
            }
        }

        row.chars.len()
    }

    fn editor_insert_row(&mut self, at: usize, s: &str) {
        if at > self.erow.len() {
            return;
        }

        for j in (at + 1)..self.number_of_rows {
            self.erow[j].idx += 1;
        }


        let mut row = EditorRow {
            size: s.len(),
            chars: s.to_string(),
            render: String::new(),
            r_size: 0,
            hl: None,
            idx: at,
            hl_open_comment: false,
        };

        row.update_row();
        self.erow.insert(at, row);
        self.number_of_rows = self.erow.len();

        // Update syntax highlighting for the new row
        self.editor_update_syntax(at);
        self.dirty += 1;
    }

    fn editor_free_row(&mut self, at: usize) {
        // check index is within bounds
        if at >= self.number_of_rows {
            return;
        }

        for j in at..self.number_of_rows {
            self.erow[j].idx -= 1;
        }

        // remove the row from the vector
        self.erow.remove(at);
        self.number_of_rows -= 1; // Update the number of rows
        self.dirty += 1; // mark the editor as modified
    }

   pub fn editor_update_syntax(&mut self, row_index: usize) {
    if row_index >= self.erow.len() {
        return;
    }

    // Early return if no syntax is set
    if self.syntax.is_none() {
        return;
    }

    let render_len = self.erow[row_index].render.len();
    let mut hl = vec![EditorHighlight::Normal as u8; render_len];

    if let Some(syntax) = self.syntax {
        // Get comment start string and its length
        let scs = syntax.single_line_comment_start;
        let mcs = syntax.multiline_comment_start;
        let mce = syntax.multiline_comment_end;
        
        let scs_len = scs.len();
        let mcs_len = mcs.len();
        let mce_len = mce.len();

        let mut i = 0;
        let mut prev_sep = true;
        let in_string: Option<char> = None;
        
        // Initialize in_comment based on previous row's state (like C code)
        let mut in_comment = if row_index > 0 {
            self.erow[row_index - 1].hl_open_comment
        } else {
            false
        };

        while i < self.erow[row_index].render.len() {
            let c = self.erow[row_index].render.as_bytes()[i] as char;

            let prev_hl = if i > 0 {
                hl[i - 1]
            } else {
                EditorHighlight::Normal as u8
            };

            // Comment highlighting - check BEFORE string highlighting
            if scs_len > 0 && in_string.is_none() && !in_comment {
                // Check if we have enough characters left and if it matches the comment start
                if i + scs_len <= self.erow[row_index].render.len() {
                    let slice = &self.erow[row_index].render[i..i + scs_len];
                    if slice == scs {
                        // Highlight the rest of the line as a comment
                        for j in i..hl.len() {
                            hl[j] = EditorHighlight::HlComment as u8;
                        }
                        break; // Done with this row
                    }
                }
            }
            
            // Multi-line comment highlighting
            if mcs_len > 0 && mce_len > 0 && in_string.is_none() {
                if in_comment {
                    hl[i] = EditorHighlight::HlMComment as u8;

                    // Check if multi-line comment ends here
                    if i + mce_len <= self.erow[row_index].render.len() && 
                       &self.erow[row_index].render[i..i + mce_len] == mce {
                        // Highlight the end marker
                        for j in i..(i + mce_len) {
                            hl[j] = EditorHighlight::HlMComment as u8;
                        }
                        i += mce_len;
                        in_comment = false;
                        prev_sep = true;
                        continue;
                    } else {
                        i += 1;
                        continue;
                    }
                } else if i + mcs_len <= self.erow[row_index].render.len() && 
                          &self.erow[row_index].render[i..i + mcs_len] == mcs {
                    // Highlight the start marker
                    for j in i..(i + mcs_len) {
                        hl[j] = EditorHighlight::HlMComment as u8;
                    }
                    i += mcs_len;
                    in_comment = true;
                    continue;
                }
            }

            // Number highlighting logic
            if syntax.flags & HL_HIGHLIGHT_NUMBERS != 0 {
                if (c.is_ascii_digit()
                    && (prev_sep || prev_hl == EditorHighlight::Number as u8))
                    || (c == '.' && prev_hl == EditorHighlight::Number as u8)
                {
                    hl[i] = EditorHighlight::Number as u8;
                    i += 1;
                    prev_sep = false;
                    continue;
                }
            }

            // Keyword highlighting logic
            if prev_sep {
                let mut keyword_found = false;
                
                // Check regular keywords
                for &keyword in syntax.keywords.iter() {
                    let klen = keyword.len();
                    let kw2 = keyword.ends_with('|');
                    let actual_klen = if kw2 { klen - 1 } else { klen };
                    
                    // Check if we have enough characters remaining
                    if i + actual_klen <= self.erow[row_index].render.len() {
                        let keyword_to_match = if kw2 { &keyword[..klen-1] } else { keyword };
                        let slice = &self.erow[row_index].render[i..i + actual_klen];
                        
                        // Check if the keyword matches and is followed by a separator
                        if slice == keyword_to_match {
                            let next_char_pos = i + actual_klen;
                            let is_end_of_line = next_char_pos >= self.erow[row_index].render.len();
                            let next_is_separator = if is_end_of_line {
                                true
                            } else {
                                let next_char = self.erow[row_index].render.as_bytes()[next_char_pos] as char;
                                Self::is_separator(next_char)
                            };
                            
                            if is_end_of_line || next_is_separator {
                                // Highlight the keyword
                                let highlight_type = if kw2 {
                                    EditorHighlight::HlKeyword2 as u8
                                } else {
                                    EditorHighlight::HlKeyword1 as u8
                                };
                                
                                for j in i..i + actual_klen {
                                    hl[j] = highlight_type;
                                }
                                
                                i += actual_klen;
                                keyword_found = true;
                                break;
                            }
                        }
                    }
                }
                
                // Check type keywords
                if !keyword_found {
                    for &type_keyword in syntax.types.iter() {
                        let klen = type_keyword.len();
                        
                        // Check if we have enough characters remaining
                        if i + klen <= self.erow[row_index].render.len() {
                            let slice = &self.erow[row_index].render[i..i + klen];
                            
                            // Check if the type keyword matches and is followed by a separator
                            if slice == type_keyword {
                                let next_char_pos = i + klen;
                                let is_end_of_line = next_char_pos >= self.erow[row_index].render.len();
                                let next_is_separator = if is_end_of_line {
                                    true
                                } else {
                                    let next_char = self.erow[row_index].render.as_bytes()[next_char_pos] as char;
                                    Self::is_separator(next_char)
                                };
                                
                                if is_end_of_line || next_is_separator {
                                    // Highlight the type keyword
                                    for j in i..i + klen {
                                        hl[j] = EditorHighlight::HlKeyword2 as u8;
                                    }
                                    
                                    i += klen;
                                    keyword_found = true;
                                    break;
                                }
                            }
                        }
                    }
                }
                
                if keyword_found {
                    prev_sep = false;
                    continue;
                }
            }

            prev_sep = Self::is_separator(c);
            i += 1;
        }
        
        // Track if the comment state changed (like C code)
        let changed = self.erow[row_index].hl_open_comment != in_comment;
        self.erow[row_index].hl_open_comment = in_comment;
        
        // If state changed, update the next row recursively
        if changed && row_index + 1 < self.erow.len() {
            self.editor_update_syntax(row_index + 1);
        }
    }

    self.erow[row_index].hl = Some(hl);
}

    fn editor_select_syntax_highlight(&mut self) {
        //reset syntax to Null
        self.syntax = None;

        //if filename is not set exit early
        let filename = match &self.filename {
            Some(name) => name, // this is now a &String
            None => return,
        };

        // Extract the file extension
        let ext = filename.rfind('.').map(|pos| &filename[pos..]);

        //iterate over the syntax database
        for syntax in HLDB.iter() {
            for &pattern in syntax.filematch.iter() {
                let is_ext = pattern.starts_with('.');
                if (is_ext && ext.is_some() && ext.unwrap() == pattern)
                    || (!is_ext && filename.contains(pattern))
                {
                    self.syntax = Some(syntax);
                    //Re-highlight the entire file
                    for i in 0..self.erow.len() {
                        self.editor_update_syntax(i);
                    }
                    return;
                }
            }
        }
    }

    fn update_all_syntax(&mut self) {
        for i in 0..self.erow.len() {
            self.editor_update_syntax(i);
        }
    }

    fn editor_insert_new_line(&mut self) {
        if self.cx == 0 {
            // Case: Cursor at beginning of line → insert empty line before
            self.editor_insert_row(self.cy, "");
        } else {
            // Case: Split line at self.cx
            let current_row = &mut self.erow[self.cy];
            let right = current_row.chars[self.cx..].to_string(); // right half

            // Truncate current row to the left half
            current_row.chars.truncate(self.cx);
            current_row.size = self.cx;
            current_row.update_row();

            // Insert new row after current with right half
            self.editor_insert_row(self.cy + 1, &right);
        }

        self.cy += 1;
        self.cx = 0;
    }

    fn editor_insert_char(&mut self, c: char) {
        if self.cy == self.number_of_rows {
            self.editor_insert_row(self.number_of_rows, "");
        }

        self.erow[self.cy].insert_char(self.cx, c);

        // Update syntax highlighting for the modified row
        self.editor_update_syntax(self.cy);

        self.cx += 1;
        self.dirty += 1;
    }

    fn editor_del_char(&mut self) {
        if self.cy >= self.number_of_rows {
            return;
        }

        if self.cx == 0 && self.cy == 0 {
            return;
        }

        if self.cx > 0 {
            self.cx -= 1;
            self.erow[self.cy].delete_char(self.cx);
            self.editor_update_syntax(self.cy);
            self.dirty += 1;
        } else {
            let current_row = self.erow.remove(self.cy);
            self.cy -= 1;
            let prev_row = &mut self.erow[self.cy];
            let prev_row_len = prev_row.size;

            prev_row.append_string(&current_row.chars);
            self.editor_update_syntax(self.cy);

            self.cx = prev_row_len;
            self.number_of_rows -= 1;
            self.dirty += 1;
        }
    }

    fn editor_row_to_string(&self) -> String {
        let mut total_len = 0;

        // compute the total length of all rows
        for row in &self.erow {
            total_len += row.chars.len() + 1;
        }

        //pre-allocate string with the total length
        let mut buffer = String::with_capacity(total_len);
        //push each line and new line
        for row in &self.erow {
            buffer.push_str(&row.chars);
            buffer.push('\n');
        }
        //return the string
        buffer
    }

    // Open the editor and initialize the first row
    fn editor_open(&mut self, filename: &str) -> io::Result<()> {
        // Open the file and read its contents
        self.filename = Some(filename.to_string());

        let file = File::open(filename)?;
        let mut reader = BufReader::new(file);

        //read the first line
        let mut line = String::new();

        while reader.read_line(&mut line)? > 0 {
            // Trim trailing newline characters
            let trimmed_line = line.trim_end_matches(|c| c == '\n' || c == '\r');
            // Create a new row and add it to the editor
            self.editor_insert_row(self.number_of_rows, trimmed_line);
            line.clear(); // Clear the line for the next read
        }

        self.dirty = 0; // Reset dirty flag
                        //set syntax highlighting based on filename
        self.editor_select_syntax_highlight();

        Ok(())
    }

    fn editor_save(&mut self) {
    let filename = match &self.filename {
        Some(name) => name.clone(),
        None => {
            // Pass None for callback since we don't need incremental behavior for filename input
            if let Some(name) = self.editor_prompt(
                "Save as: (ESC to cancel)",
                None::<fn(&mut Self, &str, EditorKey)>,
            ) {
                self.filename = Some(name.clone());
                //update syntax highlight for new filename
                self.editor_select_syntax_highlight();
                name
            } else {
                self.editor_set_status_msg("Save aborted");
                return;
            }
        }
    };

    let buffer = self.editor_row_to_string();
    let len = buffer.len();

    // Use std::fs::write for simpler file writing
    match std::fs::write(&filename, buffer.as_bytes()) {
        Ok(()) => {
            // Reset dirty flag and show success message
            self.dirty = 0;
            self.editor_set_status_msg(&format!("{} bytes written to disk", len));
        }
        Err(e) => {
            self.editor_set_status_msg(&format!("Can't save! I/O error: {}", e));
        }
    }
}

    pub fn editor_find(&mut self) {
        let saved_cy = self.cy;
        let saved_cx = self.cx;
        let saved_row_off = self.row_off;
        let saved_col_off = self.col_off;

        // Search state (shared across callback invocations)
        let mut last_match: Option<usize> = None;
        let mut direction: i32 = 1;

        let search_callback = move |editor: &mut Self, query: &str, key: EditorKey| {
            // Restore highlights when search is cancelled or completed
            match key {
                EditorKey::EnterKey | EditorKey::Escape => {
                    editor.restore_highlight(); // Restore highlights when done
                    last_match = None;
                    direction = 1;
                    return;
                }
                EditorKey::ArrowRight | EditorKey::ArrowDown => direction = 1,
                EditorKey::ArrowLeft | EditorKey::ArrowUp => direction = -1,
                _ => {
                    last_match = None;
                    direction = 1;
                }
            }

            if query.is_empty() || editor.erow.is_empty() {
                return;
            }

            if last_match.is_none() {
                direction = 1;
            }

            let row_count = editor.erow.len();
            let mut current = last_match.unwrap_or(0);

            // Wraparound search loop
            for _ in 0..row_count {
                current = if direction == 1 {
                    (current + 1) % row_count
                } else {
                    if current == 0 {
                        row_count - 1
                    } else {
                        current - 1
                    }
                };

                let row = &editor.erow[current];
                if let Some(match_index) = row.render.find(query) {
                    last_match = Some(current);
                    editor.cy = current;
                    editor.cx = editor.editor_row_rx_to_cx(row, match_index);
                    editor.row_off = editor.number_of_rows;

                    // Save current highlights before applying match highlighting
                    editor.save_highlight(current);

                    // Apply match highlighting
                    editor.erow[current].highlight_match(match_index, query);
                    break;
                }
            }
        };

        // ✅ Prompt message gives the user clear search instructions
        if self
            .editor_prompt("Search: (Use ESC/Arrows/Enter)", Some(search_callback))
            .is_none()
        {
            // Restore original cursor position if search was cancelled
            self.cy = saved_cy;
            self.cx = saved_cx;
            self.row_off = saved_row_off;
            self.col_off = saved_col_off;
        }
    }

    // Enable raw mode for terminal input
    fn enable_raw_mode(&mut self, fd: i32) -> io::Result<()> {
        // Store original termios first
        let original_termios = Termios::from_fd(fd)?;
        self.original_termios = Some(original_termios.clone());

        // Modify a copy for raw mode
        let mut raw = original_termios;
        raw.c_iflag &= !(ICRNL | BRKINT | INPCK | ISTRIP | IXON);
        raw.c_oflag &= !(OPOST);
        raw.c_cflag |= CS8;
        raw.c_lflag &= !(ECHO | ICANON | ISIG | IEXTEN);
        raw.c_cc[VMIN] = 0;
        raw.c_cc[VTIME] = 1;

        tcsetattr(fd, TCSAFLUSH, &raw)?;
        Ok(())
    }

    // Disable raw mode and restore original terminal settings
    fn disable_raw_mode(&self, fd: i32) -> io::Result<()> {
        if let Some(ref termios) = self.original_termios {
            tcsetattr(fd, TCSAFLUSH, termios)?;
        }
        Ok(())
    }

    fn editor_scroll(&mut self) {
        if self.cy < self.number_of_rows {
            let row = &self.erow[self.cy];
            self.rx = self.editor_row_cx_to_rx(row, self.cx);
        }

        if self.cy < self.row_off {
            self.row_off = self.cy;
        }

        if self.cy >= self.row_off + self.screen_rows {
            self.row_off = self.cy - self.screen_rows + 1;
        }

        if self.rx < self.col_off {
            self.col_off = self.rx;
        }

        if self.rx >= self.col_off + self.screen_cols {
            self.col_off = self.rx - self.screen_cols + 1;
        }
    }

    // Draw the tildes for empty lines
    fn draw_rows(&self, ab: &mut AppendBuffer) -> io::Result<()> {
    for y in 0..self.screen_rows {
        let file_row = y + self.row_off;
        if file_row >= self.number_of_rows {
            // Welcome message logic (unchanged)
            if self.number_of_rows == 0 && y == self.screen_rows / 3 {
                let welcome = format!("Kibi Editor -- version {}", VERSION);
                let mut welcomelen = welcome.len();

                if welcomelen > self.screen_cols {
                    welcomelen = self.screen_cols;
                }

                let padding = (self.screen_cols - welcomelen) / 2;
                if padding > 0 {
                    ab.append(b"~");
                    for _ in 1..padding {
                        ab.append(b" ");
                    }
                }
                ab.append(&welcome.as_bytes()[..welcomelen]);
            } else {
                ab.append(b"~");
            }
        } else {
            // Draw the row with proper highlighting
            let row = &self.erow[file_row];
            
            // Handle horizontal scrolling
            let start = self.col_off.min(row.render.len());
            let mut len = row.render.len().saturating_sub(self.col_off);
            if len > self.screen_cols {
                len = self.screen_cols;
            }
            
            let end = start + len;
            let visible = &row.render[start..end];
            
            if let Some(ref hl) = row.hl {
                let mut current_color: Option<u8> = None;
                
                for (j, ch) in visible.chars().enumerate() {
                    let hl_index = start + j;
                    let highlight_type = hl.get(hl_index)
                        .copied()
                        .unwrap_or(EditorHighlight::Normal as u8);
                    
                    if ch.is_ascii_control() {
                        let sym = if (ch as u8) <= 26 {
                            (b'@' + ch as u8) as char
                        } else {
                            '?'
                        };
                        
                        ab.append(b"\x1b[7m"); // Inverted colors
                        ab.append_char(sym);
                        ab.append(b"\x1b[m"); // Reset
                        
                        // Restore color if we had one
                        if let Some(color) = current_color {
                            let color_sequence = format!("\x1b[{}m", color);
                            ab.append(color_sequence.as_bytes());
                        }
                    } else if highlight_type == EditorHighlight::Normal as u8 {
                        if current_color.is_some() {
                            ab.append(b"\x1b[39m"); // Reset to default color
                            current_color = None;
                        }
                        ab.append_char(ch);
                    } else {
                        let color = self.highlight_to_color(highlight_type);
                        if current_color != Some(color) {
                            let ansi_code = format!("\x1b[{}m", color);
                            ab.append(ansi_code.as_bytes());
                            current_color = Some(color);
                        }
                        ab.append_char(ch);
                    }
                }
                
                // Reset color at end of line
                if current_color.is_some() {
                    ab.append(b"\x1b[39m");
                }
            } else {
                // No highlighting available, just append the visible text
                ab.append(visible.as_bytes());
            }
        }

        // Clear the rest of the line and add newline
        ab.append(b"\x1b[K");
        if y < self.screen_rows - 1 {
            ab.append(b"\r\n");
        }
    }

    Ok(())
}

    fn editor_draw_status_bar(&self, ab: &mut AppendBuffer) {
        // display inverted colors
        ab.append(b"\x1b[7m");

        let filename_display = self
            .filename
            .as_ref()
            .map(|f| f.as_str())
            .unwrap_or("No File");

        let modified = if self.dirty > 0 { "(Modified)" } else { "" };

        let filetype_display = match self.syntax {
            Some(syntax) => syntax.filetype,
            None => "no ft",
        };

        let r_status = format!(
            "{} | {}/{}",
            filetype_display,
            self.cy + 1,
            self.number_of_rows
        );

        //format the status string filename
        let mut status = format!(
            "{:.20} - {} lines {}",
            filename_display, self.number_of_rows, modified
        );

        //trim the string if it exceeds the screen widths
        if status.len() > self.screen_cols {
            status.truncate(self.screen_cols);
        }

        //right align the status string
        let rstatus = format!("{}/{}", self.cy + 1, self.number_of_rows);
        let rlen = rstatus.len();

        let mut len = status.len();
        while len < self.screen_cols {
            if self.screen_cols - len == rlen {
                ab.append(rstatus.as_bytes());
                break;
            } else {
                ab.append(b" ");
                len += 1;
            }
        }

        // append the status string to the buffer
        ab.append(status.as_bytes());

        // reset text format
        ab.append(b"\x1b[m"); // reset text format
        ab.append(b"\r\n"); // Move to the next line
    }

    fn editor_draw_message_bar(&self, ab: &mut AppendBuffer) {
        // Clear the current line
        ab.append(b"\x1b[K");

        let elapsed = self.status_msg_time.elapsed().unwrap_or_default();
        if !self.status_msg.is_empty() && elapsed < Duration::from_secs(5) {
            // Truncate message if it's wider than the screen
            let msg = if self.status_msg.len() > self.screen_cols {
                &self.status_msg[..self.screen_cols]
            } else {
                &self.status_msg
            };
            ab.append(msg.as_bytes());
        }
    }

    // Refresh the screen
    fn refresh_screen(&mut self) -> io::Result<()> {
        self.editor_scroll();
        let mut ab = AppendBuffer::new();

        // Clear screen and position cursor
        ab.append(b"\x1b[?25l\x1b[H");

        // Draw the tildes
        self.draw_rows(&mut ab)?;
        self.editor_draw_status_bar(&mut ab);
        self.editor_draw_message_bar(&mut ab);
        //allow user position cursor
        let cursor_position = format!(
            "\x1b[{};{}H",
            self.cy - self.row_off + 1,
            self.rx - self.col_off + 1
        );
        ab.append(cursor_position.as_bytes());

        // show the cursor again
        ab.append(b"\x1b[?25h");
        // Output everything in one go
        ab.write_all()?;

        Ok(())
    }

    fn editor_set_status_msg(&mut self, msg: impl std::fmt::Display) {
        self.status_msg = msg.to_string();
        self.status_msg_time = SystemTime::now();
    }

    // Read a key from stdin
    fn read_key(&self) -> io::Result<EditorKey> {
    let stdin = io::stdin();
    let mut handle = stdin.lock();

    let mut c = [0u8; 1];
    let bytes_read = handle.read(&mut c)?;

    if bytes_read == 0 {
        return Ok(EditorKey::Other(0));
    }

    // Handle Enter key
    if c[0] == b'\r' || c[0] == b'\n' {
        return Ok(EditorKey::EnterKey);
    }

    // Handle Backspace key
    if c[0] == 127 || c[0] == 8 {
        return Ok(EditorKey::Backspace);
    }

    // Handle Ctrl combinations first (before WASD)
    if c[0] == ctrl_key(b'q') {
        return Ok(EditorKey::CtrlQ);
    }
    if c[0] == ctrl_key(b's') {
        eprintln!("DEBUG: Ctrl+S detected"); // Add this line
        return Ok(EditorKey::CtrlS);
    }
    if c[0] == ctrl_key(b'f') {
        return Ok(EditorKey::CtrlF);
    }
    if c[0] == ctrl_key(b'h') {
        return Ok(EditorKey::CtrlH);
    }
    if c[0] == ctrl_key(b'l') {
        return Ok(EditorKey::CtrlL);
    }

    // Handle escape sequences
    if c[0] == b'\x1b' {
        let mut seq = [0u8; 2];
        let mut idx = 0;
        
        while idx < 2 {
            match handle.read(&mut seq[idx..idx + 1]) {
                Ok(0) => break,
                Ok(_) => idx += 1,
                Err(_) => break,
            }
        }

        if idx == 0 {
            return Ok(EditorKey::Escape);
        }

        if seq[0] == b'[' && idx > 1 {
            if seq[1].is_ascii_digit() {
                let mut third = [0u8; 1];
                let read_third = handle.read(&mut third).unwrap_or(0);

                if read_third > 0 && third[0] == b'~' {
                    return match seq[1] {
                        b'1' | b'7' => Ok(EditorKey::HomeKey),
                        b'3' => Ok(EditorKey::Delete),
                        b'4' | b'8' => Ok(EditorKey::EndKey),
                        b'5' => Ok(EditorKey::PageUp),
                        b'6' => Ok(EditorKey::PageDown),
                        _ => Ok(EditorKey::Escape),
                    };
                }
            } else {
                return match seq[1] {
                    b'A' => Ok(EditorKey::ArrowUp),
                    b'B' => Ok(EditorKey::ArrowDown),
                    b'C' => Ok(EditorKey::ArrowRight),
                    b'D' => Ok(EditorKey::ArrowLeft),
                    b'H' => Ok(EditorKey::HomeKey),
                    b'F' => Ok(EditorKey::EndKey),
                    _ => Ok(EditorKey::Escape),
                };
            }
        } else if seq[0] == b'O' && idx > 1 {
            return match seq[1] {
                b'H' => Ok(EditorKey::HomeKey),
                b'F' => Ok(EditorKey::EndKey),
                _ => Ok(EditorKey::Escape),
            };
        }

        return Ok(EditorKey::Escape);
    }

    // Only treat WASD as movement if they're not part of normal text input
    // You might want to add a mode for this or remove WASD movement entirely
    // For now, I'm commenting this out to prevent conflicts:
    /*
    match c[0] {
        b'w' => Ok(EditorKey::ArrowUp),
        b's' => Ok(EditorKey::ArrowDown),
        b'a' => Ok(EditorKey::ArrowLeft),
        b'd' => Ok(EditorKey::ArrowRight),
        _ => Ok(EditorKey::Other(c[0])),
    }
    */
    
    // Just return the character as-is
    Ok(EditorKey::Other(c[0]))
}

    fn editor_prompt<F>(&mut self, prompt: &str, mut callback: Option<F>) -> Option<String>
    where
        F: FnMut(&mut Self, &str, EditorKey),
    {
        let mut buf = String::new();

        loop {
            self.editor_set_status_msg(&format!("{}{}", prompt, buf));
            if let Err(_) = self.refresh_screen() {
                return None;
            }

            let c = match self.read_key() {
                Ok(key) => key,
                Err(_) => return None,
            };

            match c {
                EditorKey::EnterKey => {
                    if !buf.is_empty() {
                        self.editor_set_status_msg("");
                        return Some(buf);
                    }
                }
                EditorKey::Backspace | EditorKey::CtrlH | EditorKey::Delete => {
                    if !buf.is_empty() {
                        buf.pop();
                    }
                }
                EditorKey::Escape => {
                    self.editor_set_status_msg("");
                    return None;
                }
                EditorKey::Other(ch) => {
                    if ch.is_ascii_graphic() || ch == b' ' {
                        buf.push(ch as char);
                    }
                }
                _ => {}
            }

            // Call callback after each keypress
            if let Some(ref mut cb) = callback {
                cb(self, &buf, c);
            }
        }
    }

    // move the cursor depending on the key pressed
    pub fn editor_move_cursor(&mut self, key: EditorKey) {
    let current_row = if self.cy < self.number_of_rows {
        Some(&self.erow[self.cy])
    } else {
        None
    };

    match key {
        EditorKey::ArrowLeft => {
            if self.cx > 0 {
                self.cx -= 1;
            } else if self.cy > 0 {
                // Move to the end of the previous line
                self.cy -= 1;
                self.cx = self.erow[self.cy].size;
            }
        }

        EditorKey::ArrowRight => {
            if let Some(row) = current_row {
                if self.cx < row.size {
                    self.cx += 1;
                }
                // let user explicitly press Enter
                // or use End key to go to end of line
            } else if self.cy < self.number_of_rows {
                // If we're past the last row, don't move
                return;
            }
        }

        EditorKey::ArrowUp => {
            if self.cy > 0 {
                self.cy -= 1;
            }
        }

        EditorKey::ArrowDown => {
            if self.cy < self.number_of_rows {
                self.cy += 1;
            }
        }
        _ => {}
    }

    // Snap cursor to end of line if it's beyond the line length
    let current_row = if self.cy < self.number_of_rows {
        Some(&self.erow[self.cy])
    } else {
        None
    };

    if let Some(row) = current_row {
        if self.cx > row.size {
            self.cx = row.size;
        }
    } else {
        self.cx = 0;
    }
}


    // Fixed process_keypress function with no unreachable patterns
    fn process_keypress(&mut self) -> io::Result<bool> {
        let c = self.read_key()?;

        match c {
            EditorKey::EnterKey => {
                self.editor_insert_new_line();
            }
            EditorKey::CtrlQ => {
                if self.dirty > 0 && self.quit_times > 0 {
                    self.editor_set_status_msg(&format!(
                        "WARNING!!! File has unsaved changes. Press Ctrl-Q {} more times to quit.",
                        self.quit_times
                    ));
                    self.quit_times -= 1;
                    return Ok(true);
                }

                self.refresh_screen()?; // or refresh_screen
                return Ok(false); // exit
            }

            EditorKey::CtrlS => {
                eprintln!("DEBUG: Ctrl+S pressed"); // Add this line
                self.editor_save();
            }

            EditorKey::CtrlF => self.editor_find(),

            EditorKey::PageUp => {
                // move the cursor up by the number of screen rows
                self.cy = self.row_off;
            }
            EditorKey::PageDown => {
                // Move the cursor down by the number of screen rows
                self.cy = self.row_off + self.screen_rows - 1;
                if self.cy > self.number_of_rows {
                    self.cy = self.number_of_rows;
                }
            }

            EditorKey::HomeKey => {
                //move cursor to the beginning of the line
                self.cx = 0
            }
            EditorKey::EndKey => {
                // move cursor to the end of the line
                if self.cy < self.number_of_rows {
                    self.cx = self.erow[self.cy].size;
                }
            }

            EditorKey::Delete => {
                if self.cy >= self.number_of_rows {
                    return Ok(true); // Nothing to delete
                }

                // Check if we're deleting a character within the current line
                if self.cx < self.erow[self.cy].chars.len() {
                    // Delete character at current cursor position
                    self.erow[self.cy].delete_char(self.cx);
                    self.dirty += 1;
                } else if self.cx == self.erow[self.cy].chars.len()
                    && self.cy < self.number_of_rows - 1
                {
                    // At end of line, join with next line
                    let next_row = self.erow.remove(self.cy + 1);
                    let current_row = &mut self.erow[self.cy];
                    current_row.append_string(&next_row.chars);
                    self.number_of_rows -= 1;
                    self.dirty += 1;
                }
            }

            EditorKey::Backspace | EditorKey::CtrlH => {
                // Backspace: delete character before cursor
                self.editor_del_char();
            }

            EditorKey::ArrowUp
            | EditorKey::ArrowDown
            | EditorKey::ArrowRight
            | EditorKey::ArrowLeft => {
                //move the cursor based on the key pressed
                self.editor_move_cursor(c);
            }

            // display printable characters
            EditorKey::Other(byte) => {
                if byte.is_ascii_graphic() || byte == b' ' {
                    self.editor_insert_char(byte as char);
                }
            }

            EditorKey::CtrlL | EditorKey::Escape => {
                // Do nothing.
            }
        }

        if c != EditorKey::CtrlQ {
            self.quit_times = QUIT_TIMES;
        }

        Ok(true) 
    }
    // Mapping raw key (from input) to enum
    /*fn parse_key(byte: u8) -> EditorKey {
        match byte {
            0x13 => EditorKey::CtrlS,
            _ => EditorKey::Other(byte),
        }
    } */
}

fn main() -> io::Result<()> {
    // Create the editor instance
    let args: Vec<String> = std::env::args().collect();
    let stdin_fd = io::stdin().as_raw_fd();
    let mut editor = match EditorConfig::new() {
        Ok(editor) => editor,
        Err(e) => die(&format!("Failed to initialize editor: {}", e)),
    };

    // Set the status message
    editor.editor_set_status_msg("HELP: Ctrl-S | Ctrl-Q = quit | Ctrl-F = find");

    // Open a file is provided as an argument
    if args.len() >= 2 {
        editor.editor_open(&args[1])?;
    }

    // Enable raw mode
    if let Err(e) = editor.enable_raw_mode(stdin_fd) {
        die(&format!("Failed to enable raw mode: {}", e));
    }

    // Main program loop with proper error handling
    loop {
        if let Err(e) = editor.refresh_screen() {
            die(&format!("Failed to refresh screen: {}", e));
        }

        match editor.process_keypress() {
            Ok(true) => continue, // Continue processing
            Ok(false) => break,   // Exit requested
            Err(e) => die(&format!("Error processing keypress: {}", e)),
        }
    }

    // Always restore terminal setting
    if let Err(e) = editor.disable_raw_mode(stdin_fd) {
        eprintln!("Error disabling raw mode: {}", e);
    }

    Ok(())
}
