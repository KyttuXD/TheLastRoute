//! The system file chooser, for every build and for mods to reuse.
//!
//! * Web (wasm32): the page's own chooser. `packaging/web/index.html` registers `lr_pick_open` and friends as a miniquad
//!   plugin: a hidden `<input type=file>` is made and clicked, the chosen files are read into the page, and the game polls
//!   for the answer. The bytes stay in the page until [`read`] copies them over, one file at a time.
//! * Windows and macOS: the system open dialog (the `rfd` crate), opening in the Pictures folder, else Desktop, else the
//!   home folder. Every desktop uses rfd's non-blocking async dialog, which the game polls each frame. The dialog does
//!   not run a nested loop inside the frame: the blocking macOS open panel did, and that crashed the GL view's display.
//! * Linux without a dialog backend (no desktop portal and no zenity): [`open`] says so and the caller shows a line about
//!   dropping the file on the window.
//!
//! The API is the same everywhere: [`open`] returns a [`Request`] at once, [`state`] says whether it is still pending, done
//! or cancelled, and [`count`], [`name`], [`size`] and [`read`] describe the chosen files. [`poll`] does all of it in one
//! call. Nothing here throws or panics into the game: a chooser that cannot open is an `Err` with words for the player.

/// One open chooser.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Request(u32);

/// Where a chooser stands.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    /// The chooser is open (or about to open) and nothing is chosen yet.
    Pending,
    /// Files were chosen: [`count`], [`name`], [`size`] and [`read`] answer for them.
    Done,
    /// The player closed the chooser without choosing (or it chose nothing, or the request is unknown).
    Cancelled,
}

/// One chosen file: the name and its bytes, or the reason the bytes were not taken.
#[derive(Clone, Debug, PartialEq)]
pub struct PickedFile {
    pub name: String,
    pub bytes: Result<Vec<u8>, String>,
}

/// What [`poll`] found.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Pending,
    Cancelled,
    Picked(Vec<PickedFile>),
}

/// Everything a chooser backend answers. The web page and the native registry both implement it, so [`poll_from`] (and its
/// tests) do not care which one is behind it.
trait Source {
    fn state(&mut self, id: u32) -> State;
    fn count(&mut self, id: u32) -> usize;
    fn name(&mut self, id: u32, index: usize) -> String;
    fn size(&mut self, id: u32, index: usize) -> Option<usize>;
    fn read(&mut self, id: u32, index: usize, cap: usize) -> Result<Vec<u8>, String>;
    fn release(&mut self, id: u32);
}

/// Asks the chooser what happened. Pending stays pending. Cancelled and done both end the request (it is released here);
/// done reads every chosen file, a file over `cap` bytes is reported as too big without being read.
fn poll_from(src: &mut dyn Source, id: u32, cap: usize) -> Outcome {
    match src.state(id) {
        State::Pending => Outcome::Pending,
        State::Cancelled => {
            src.release(id);
            Outcome::Cancelled
        }
        State::Done => {
            let files = (0..src.count(id))
                .map(|i| {
                    let name = src.name(id, i);
                    let bytes = match src.size(id, i) {
                        Some(n) if n > cap => Err("the file is too big".to_string()),
                        _ => src.read(id, i, cap),
                    };
                    PickedFile { name, bytes }
                })
                .collect();
            src.release(id);
            Outcome::Picked(files)
        }
    }
}

/// A request value for the tab's tests: the chooser is never asked about it, the outcome is handed over directly.
#[cfg(test)]
pub fn test_request(id: u32) -> Request {
    Request(id)
}

/// The words for a player when the chooser cannot open at all.
pub const NO_CHOOSER: &str = "No file chooser is available here. Drop a PNG on the window instead.";

// ------------------------------------------------------------------------------------------- the accept string

/// The file extensions an accept string names. `kind` is written like the HTML `accept` attribute: comma separated
/// extensions (".png") and MIME types ("image/png", "image/*"). The page uses the string as it is; the native dialog
/// filters by these extensions. Empty when nothing in it maps to an extension (then there is no filter).
pub fn extensions_of(kind: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut add = |exts: &[&str]| {
        for e in exts {
            if !out.iter().any(|x| x == e) {
                out.push((*e).to_string());
            }
        }
    };
    for token in kind.split(',').map(|t| t.trim().to_ascii_lowercase()) {
        if let Some(ext) = token.strip_prefix('.') {
            if !ext.is_empty() && ext.chars().all(|c| c.is_ascii_alphanumeric()) {
                add(&[ext]);
            }
            continue;
        }
        match token.as_str() {
            "image/png" => add(&["png"]),
            "image/jpeg" | "image/jpg" => add(&["jpg", "jpeg"]),
            "image/gif" => add(&["gif"]),
            "image/webp" => add(&["webp"]),
            "image/bmp" => add(&["bmp"]),
            "image/*" => add(&["png", "jpg", "jpeg", "gif", "bmp", "webp"]),
            "audio/ogg" => add(&["ogg"]),
            "audio/wav" | "audio/x-wav" => add(&["wav"]),
            "audio/mpeg" => add(&["mp3"]),
            "audio/*" => add(&["wav", "ogg", "mp3", "flac"]),
            "text/plain" => add(&["txt"]),
            "application/json" => add(&["json"]),
            "application/zip" => add(&["zip"]),
            _ => {}
        }
    }
    out
}

// ------------------------------------------------------------------------------------------- the public calls

/// Opens a chooser. `kind` is an accept string (see [`extensions_of`]), `multiple` allows several files, `directory`
/// chooses a folder (its files, with names relative to the folder). Returns at once; the chooser itself is [`state`]'s
/// business. `Err` carries the words for the player when no chooser can open.
pub fn open(kind: &str, multiple: bool, directory: bool) -> Result<Request, String> {
    imp::open(kind, multiple, directory).map(Request)
}

/// Where a request stands. An unknown or released request is `Cancelled`.
pub fn state(r: Request) -> State {
    imp::with(|s| s.state(r.0))
}

/// How many files a done request holds.
pub fn count(r: Request) -> usize {
    imp::with(|s| s.count(r.0))
}

/// The name of file `i`: the file name, or the path inside the chosen folder for a folder.
pub fn name(r: Request, i: usize) -> String {
    imp::with(|s| s.name(r.0, i))
}

/// The size of file `i` in bytes, when known.
pub fn size(r: Request, i: usize) -> Option<usize> {
    imp::with(|s| s.size(r.0, i))
}

/// The bytes of file `i`. A file over `cap` bytes is refused with "the file is too big" and never copied.
pub fn read(r: Request, i: usize, cap: usize) -> Result<Vec<u8>, String> {
    imp::with(|s| s.read(r.0, i, cap))
}

/// Lets go of a request (a chooser still open is ignored when it answers). [`poll`] releases by itself on cancel and done.
pub fn release(r: Request) {
    imp::with(|s| s.release(r.0));
}

/// [`state`] and, when done, every chosen file read (at most `cap` bytes each). The request is released once it answers.
pub fn poll(r: Request, cap: usize) -> Outcome {
    imp::with(|s| poll_from(s, r.0, cap))
}

/// True when [`open`] can work on this system (always on the web, Windows and macOS; on Linux when a portal or zenity is there).
pub fn available() -> bool {
    imp::available()
}

// ------------------------------------------------------------------------------------------- the web page

#[cfg(target_arch = "wasm32")]
mod imp {
    use super::{Source, State};

    extern "C" {
        /// Opens the page's file chooser (a hidden `<input type=file>` with `accept` = the kind string). Returns a request id,
        /// 0 when the page could not make one.
        fn lr_pick_open(kind: *const u8, kind_len: u32, multiple: u32, directory: u32) -> u32;
        /// 0 pending, 1 done, 2 cancelled (also an unknown id).
        fn lr_pick_state(id: u32) -> i32;
        fn lr_pick_count(id: u32) -> i32;
        /// Copies file `i`'s name into `dst` (at most `cap` bytes) and returns its byte length; nothing is copied when it is longer.
        fn lr_pick_name(id: u32, i: u32, dst: *mut u8, cap: u32) -> i32;
        /// File `i`'s size in bytes, -1 when unknown.
        fn lr_pick_size(id: u32, i: u32) -> i32;
        /// Copies file `i`'s bytes into `dst` (at most `cap`) and returns their length, -1 when they are not there.
        fn lr_pick_read(id: u32, i: u32, dst: *mut u8, cap: u32) -> i32;
        /// Forgets the request and its bytes.
        fn lr_pick_release(id: u32);
    }

    pub struct Js;

    impl Source for Js {
        fn state(&mut self, id: u32) -> State {
            // SAFETY: plain integers in, an integer out.
            match unsafe { lr_pick_state(id) } {
                0 => State::Pending,
                1 => State::Done,
                _ => State::Cancelled,
            }
        }
        fn count(&mut self, id: u32) -> usize {
            // SAFETY: as above.
            unsafe { lr_pick_count(id) }.max(0) as usize
        }
        fn name(&mut self, id: u32, index: usize) -> String {
            let mut cap = 512usize;
            for _ in 0..2 {
                let mut buf = vec![0u8; cap];
                // SAFETY: the buffer is `cap` bytes long and the page copies at most `cap` bytes into it.
                let n = unsafe { lr_pick_name(id, index as u32, buf.as_mut_ptr(), cap as u32) };
                if n < 0 {
                    break;
                }
                if n as usize <= cap {
                    buf.truncate(n as usize);
                    return String::from_utf8_lossy(&buf).into_owned();
                }
                cap = n as usize;
            }
            String::new()
        }
        fn size(&mut self, id: u32, index: usize) -> Option<usize> {
            // SAFETY: plain integers in, an integer out.
            let n = unsafe { lr_pick_size(id, index as u32) };
            (n >= 0).then_some(n as usize)
        }
        fn read(&mut self, id: u32, index: usize, cap: usize) -> Result<Vec<u8>, String> {
            let size = self.size(id, index).ok_or_else(|| "the browser did not give the file".to_string())?;
            if size > cap {
                return Err("the file is too big".to_string());
            }
            let mut buf = vec![0u8; size];
            // SAFETY: the buffer is `size` bytes long and the page copies at most that many into it.
            let n = unsafe { lr_pick_read(id, index as u32, buf.as_mut_ptr(), size as u32) };
            if n < 0 || n as usize != size {
                return Err("the browser did not give the file".to_string());
            }
            Ok(buf)
        }
        fn release(&mut self, id: u32) {
            // SAFETY: a plain integer in.
            unsafe { lr_pick_release(id) }
        }
    }

    pub fn with<T>(f: impl FnOnce(&mut dyn Source) -> T) -> T {
        f(&mut Js)
    }

    pub fn open(kind: &str, multiple: bool, directory: bool) -> Result<u32, String> {
        // SAFETY: `kind` is a live slice for the whole call and its length goes with it.
        let id = unsafe { lr_pick_open(kind.as_ptr(), kind.len() as u32, multiple as u32, directory as u32) };
        if id == 0 {
            Err("The browser would not open its file chooser. Drop a PNG on the page instead.".to_string())
        } else {
            Ok(id)
        }
    }

    pub fn available() -> bool {
        true
    }
}


// ------------------------------------------------------------------------------------------- Windows, macOS, Linux

#[cfg(not(target_arch = "wasm32"))]
mod imp {
    use super::{extensions_of, Source, State, NO_CHOOSER};
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};

    /// A running dialog, polled each frame: `None` while it is open, `Some(None)` when it was closed without a choice,
    /// `Some(Some(paths))` when files (or a folder) were chosen.
    pub struct Dialog(pub Box<dyn FnMut() -> Option<Option<Vec<PathBuf>>>>);

    /// How the dialog is started. It runs on the first `state` call after `open`, so the frame that asked for it is drawn first.
    pub type Launch = Box<dyn FnOnce() -> Dialog>;

    enum Stage {
        Queued(Launch),
        Running(Dialog),
        Done(Vec<Chosen>),
        Cancelled,
    }

    struct Entry {
        /// A folder was asked for: the dialog's answer is the folder, and the request holds the files inside it.
        directory: bool,
        stage: Stage,
    }

    pub struct Chosen {
        pub name: String,
        pub path: PathBuf,
    }

    /// The open choosers, by id.
    #[derive(Default)]
    pub struct Registry {
        next: u32,
        entries: HashMap<u32, Entry>,
    }

    /// The most files a chosen folder gives, and how deep it is searched.
    const FOLDER_FILES_MAX: usize = 512;
    const FOLDER_DEPTH_MAX: usize = 8;

    /// The files inside a chosen folder, named by their path relative to it and prefixed with its name (like the page's
    /// `webkitRelativePath`), in name order.
    pub fn folder_files(dir: &Path) -> Vec<Chosen> {
        fn walk(dir: &Path, prefix: &str, depth: usize, out: &mut Vec<Chosen>) {
            let Ok(entries) = std::fs::read_dir(dir) else { return };
            let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
            entries.sort_by_key(|e| e.file_name());
            for e in entries {
                if out.len() >= FOLDER_FILES_MAX {
                    return;
                }
                let name = format!("{prefix}/{}", e.file_name().to_string_lossy());
                let Ok(kind) = e.file_type() else { continue };
                if kind.is_file() {
                    out.push(Chosen { name, path: e.path() });
                } else if kind.is_dir() && depth < FOLDER_DEPTH_MAX {
                    walk(&e.path(), &name, depth + 1, out);
                }
            }
        }
        let root = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let mut out = Vec::new();
        walk(dir, &root, 0, &mut out);
        out
    }

    impl Registry {
        pub fn open_with(&mut self, directory: bool, launch: Launch) -> u32 {
            self.next += 1;
            self.entries.insert(self.next, Entry { directory, stage: Stage::Queued(launch) });
            self.next
        }

        /// Starts a queued dialog and takes the answer of a running one, once.
        fn advance(&mut self, id: u32) {
            let Some(entry) = self.entries.get_mut(&id) else { return };
            if matches!(entry.stage, Stage::Queued(_)) {
                if let Stage::Queued(launch) = std::mem::replace(&mut entry.stage, Stage::Cancelled) {
                    entry.stage = Stage::Running(launch());
                }
            }
            let answer = match &mut entry.stage {
                Stage::Running(dialog) => (dialog.0)(),
                _ => return,
            };
            entry.stage = match answer {
                None => return,
                Some(None) => Stage::Cancelled,
                Some(Some(paths)) => {
                    let files: Vec<Chosen> = if entry.directory {
                        paths.iter().flat_map(|p| folder_files(p)).collect()
                    } else {
                        paths
                            .into_iter()
                            .map(|p| Chosen { name: p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), path: p })
                            .collect()
                    };
                    if files.is_empty() {
                        Stage::Cancelled
                    } else {
                        Stage::Done(files)
                    }
                }
            };
        }

        fn files(&self, id: u32) -> Option<&Vec<Chosen>> {
            match self.entries.get(&id) {
                Some(Entry { stage: Stage::Done(files), .. }) => Some(files),
                _ => None,
            }
        }
    }

    impl Source for Registry {
        fn state(&mut self, id: u32) -> State {
            self.advance(id);
            match self.entries.get(&id).map(|e| &e.stage) {
                Some(Stage::Queued(_)) | Some(Stage::Running(_)) => State::Pending,
                Some(Stage::Done(_)) => State::Done,
                Some(Stage::Cancelled) | None => State::Cancelled,
            }
        }
        fn count(&mut self, id: u32) -> usize {
            self.files(id).map_or(0, Vec::len)
        }
        fn name(&mut self, id: u32, index: usize) -> String {
            self.files(id).and_then(|f| f.get(index)).map(|f| f.name.clone()).unwrap_or_default()
        }
        fn size(&mut self, id: u32, index: usize) -> Option<usize> {
            let file = self.files(id)?.get(index)?;
            std::fs::metadata(&file.path).ok().map(|m| m.len() as usize)
        }
        fn read(&mut self, id: u32, index: usize, cap: usize) -> Result<Vec<u8>, String> {
            let file = self
                .files(id)
                .ok_or_else(|| "nothing was chosen".to_string())?
                .get(index)
                .ok_or_else(|| "that file is not in the list".to_string())?;
            crate::shared::skins::read_capped(&file.path, cap).map_err(|e| e.to_string())
        }
        fn release(&mut self, id: u32) {
            self.entries.remove(&id);
        }
    }

    thread_local! {
        static REGISTRY: RefCell<Registry> = RefCell::new(Registry::default());
    }

    pub fn with<T>(f: impl FnOnce(&mut dyn Source) -> T) -> T {
        REGISTRY.with(|r| match r.try_borrow_mut() {
            Ok(mut reg) => f(&mut *reg),
            // nothing here calls back into the registry; a nested call would see an empty one
            Err(_) => f(&mut Registry::default()),
        })
    }

    // ---- the real dialog

    /// Where the dialog starts: Pictures, else Desktop, else the home folder. `exists` says whether a folder is there.
    pub fn start_dir_from(home: Option<PathBuf>, exists: &dyn Fn(&Path) -> bool) -> Option<PathBuf> {
        let home = home?;
        [
            home.join("Pictures"),
            home.join("OneDrive").join("Pictures"),
            home.join("Desktop"),
            home.join("OneDrive").join("Desktop"),
            home.clone(),
        ]
        .into_iter()
        .find(|d| exists(d))
    }

    /// [`start_dir_from`] for this user.
    pub fn start_dir() -> Option<PathBuf> {
        let key = if cfg!(target_os = "windows") { "USERPROFILE" } else { "HOME" };
        let home = std::env::var_os(key).filter(|v| !v.is_empty()).map(PathBuf::from);
        start_dir_from(home, &|d| d.is_dir())
    }

    /// Linux has no dialog of its own: it needs the desktop portal (a session bus) or zenity.
    #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "dragonfly", target_os = "netbsd", target_os = "openbsd"))]
    pub fn available() -> bool {
        let bus = std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_some_and(|v| !v.is_empty());
        let zenity = std::env::var_os("PATH").is_some_and(|path| std::env::split_paths(&path).any(|d| d.join("zenity").is_file()));
        bus || zenity
    }

    #[cfg(not(any(target_os = "linux", target_os = "freebsd", target_os = "dragonfly", target_os = "netbsd", target_os = "openbsd")))]
    pub fn available() -> bool {
        true
    }

    fn title(directory: bool) -> &'static str {
        if directory {
            "Choose a folder"
        } else {
            "Choose a file"
        }
    }

    /// The real dialog for a request, on every desktop. It is rfd's async dialog, polled by the frame loop: the open panel
    /// is shown without a nested run loop, so nothing is drawn from inside the GL view's display (macOS: a sheet on the
    /// main window; Windows and Linux: the dialog runs on a thread of its own). A synchronous `pick_file` on macOS runs
    /// AppKit's modal loop inside the frame callback, which aborted the game in `CA::Transaction::commit`.
    #[cfg(not(lr_macos_legacy))]
    fn real_dialog(kind: String, multiple: bool, directory: bool) -> Dialog {
        use std::future::Future;
        use std::pin::Pin;
        use std::task::{Context, Poll};
        let mut dialog = rfd::AsyncFileDialog::new().set_title(title(directory));
        if let Some(dir) = start_dir() {
            dialog = dialog.set_directory(dir);
        }
        let exts = extensions_of(&kind);
        if !exts.is_empty() && !directory {
            dialog = dialog.add_filter("Accepted files", &exts);
        }
        let mut future: Pin<Box<dyn Future<Output = Option<Vec<PathBuf>>>>> = if directory {
            Box::pin(async move { dialog.pick_folder().await.map(|h| vec![h.path().to_path_buf()]) })
        } else if multiple {
            Box::pin(async move { dialog.pick_files().await.map(|hs| hs.iter().map(|h| h.path().to_path_buf()).collect()) })
        } else {
            Box::pin(async move { dialog.pick_file().await.map(|h| vec![h.path().to_path_buf()]) })
        };
        let waker = crate::shared::task::noop_waker();
        Dialog(Box::new(move || match future.as_mut().poll(&mut Context::from_waker(&waker)) {
            Poll::Pending => None,
            Poll::Ready(answer) => Some(answer),
        }))
    }

    pub fn open(kind: &str, multiple: bool, directory: bool) -> Result<u32, String> {
        if !available() {
            return Err(NO_CHOOSER.to_string());
        }
        let kind = kind.to_string();
        Ok(REGISTRY.with(|r| r.borrow_mut().open_with(directory, Box::new(move || real_dialog(kind, multiple, directory)))))
    }

    /// The old-Mac build's dialog (OS X 10.5 and up): AppKit's own open panel, asked through the Objective-C runtime
    /// with calls that exist since 10.5. The `rfd` crate needs system calls from 10.7 and is left out of that build
    /// (packaging/macos/legacy/stage-dependencies.sh). The panel runs modally: the old systems draw the GL view
    /// without a layer, so the nested loop that aborted the game on current systems (see above) does no harm there.
    #[cfg(lr_macos_legacy)]
    fn real_dialog(kind: String, multiple: bool, directory: bool) -> Dialog {
        let mut answer = Some(legacy_panel(&extensions_of(&kind), multiple, directory, start_dir()));
        Dialog(Box::new(move || answer.take().or(Some(None))))
    }

    #[cfg(lr_macos_legacy)]
    fn legacy_panel(exts: &[String], multiple: bool, directory: bool, start: Option<PathBuf>) -> Option<Vec<PathBuf>> {
        use core::ffi::{c_char, c_long, c_void};
        use std::ffi::{CStr, CString};
        type Id = *mut c_void;
        #[link(name = "objc")]
        extern "C" {
            fn objc_getClass(name: *const c_char) -> Id;
            fn sel_registerName(name: *const c_char) -> Id;
            fn objc_msgSend();
        }
        // `objc_msgSend` is called through a pointer typed to each call's signature (app/pacing.rs does the same)
        type Msg0 = unsafe extern "C" fn(Id, Id) -> Id;
        type Msg1 = unsafe extern "C" fn(Id, Id, Id) -> Id;
        type MsgBool = unsafe extern "C" fn(Id, Id, i8);
        type MsgLong = unsafe extern "C" fn(Id, Id) -> c_long;
        type MsgAt = unsafe extern "C" fn(Id, Id, std::ffi::c_ulong) -> Id;
        type MsgCStr = unsafe extern "C" fn(Id, Id, *const c_char) -> Id;
        type MsgResponds = unsafe extern "C" fn(Id, Id, Id) -> i8;
        unsafe {
            let raw = objc_msgSend as *const ();
            let (m0, m1, mb, ml, mat, mc, mr): (Msg0, Msg1, MsgBool, MsgLong, MsgAt, MsgCStr, MsgResponds) = (
                std::mem::transmute(raw),
                std::mem::transmute(raw),
                std::mem::transmute(raw),
                std::mem::transmute(raw),
                std::mem::transmute(raw),
                std::mem::transmute(raw),
                std::mem::transmute(raw),
            );
            let sel = |name: &str| {
                let c = CString::new(name).expect("a selector has no NUL");
                sel_registerName(c.as_ptr())
            };
            let class = |name: &str| {
                let c = CString::new(name).expect("a class name has no NUL");
                objc_getClass(c.as_ptr())
            };
            let nsstring = |text: &str| -> Id {
                let c = CString::new(text.replace('\0', "")).expect("NUL removed");
                mc(class("NSString"), sel("stringWithUTF8String:"), c.as_ptr())
            };
            let responds = |object: Id, name: &str| mr(object, sel("respondsToSelector:"), sel(name)) != 0;
            let panel_class = class("NSOpenPanel");
            if panel_class.is_null() {
                return None;
            }
            let panel = m0(panel_class, sel("openPanel"));
            if panel.is_null() {
                return None;
            }
            mb(panel, sel("setCanChooseFiles:"), i8::from(!directory));
            mb(panel, sel("setCanChooseDirectories:"), i8::from(directory));
            mb(panel, sel("setAllowsMultipleSelection:"), i8::from(multiple));
            if !exts.is_empty() && !directory {
                let types = m0(class("NSMutableArray"), sel("array"));
                for e in exts {
                    m1(types, sel("addObject:"), nsstring(e));
                }
                m1(panel, sel("setAllowedFileTypes:"), types);
            }
            if let Some(dir) = start.as_ref().and_then(|d| d.to_str()) {
                if responds(panel, "setDirectory:") {
                    m1(panel, sel("setDirectory:"), nsstring(dir));
                }
            }
            // NSOKButton / NSModalResponseOK is 1
            if ml(panel, sel("runModal")) != 1 {
                return None;
            }
            // `filenames` is the 10.5 call; later systems that dropped it answer to `URLs`
            let (list, are_urls) = if responds(panel, "filenames") {
                (m0(panel, sel("filenames")), false)
            } else {
                (m0(panel, sel("URLs")), true)
            };
            if list.is_null() {
                return None;
            }
            let count = ml(list, sel("count")).max(0) as std::ffi::c_ulong;
            let mut out = Vec::new();
            for i in 0..count {
                let mut item = mat(list, sel("objectAtIndex:"), i);
                if are_urls && !item.is_null() {
                    item = m0(item, sel("path"));
                }
                if item.is_null() {
                    continue;
                }
                let text = m0(item, sel("UTF8String")) as *const c_char;
                if !text.is_null() {
                    out.push(PathBuf::from(CStr::from_ptr(text).to_string_lossy().into_owned()));
                }
            }
            (!out.is_empty()).then_some(out)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_accept_string_gives_the_native_filter_its_extensions() {
        assert_eq!(extensions_of("image/png,.png"), ["png"], "the same extension from a type and a suffix is listed once");
        assert_eq!(extensions_of(" image/jpeg , .WEBP "), ["jpg", "jpeg", "webp"]);
        assert_eq!(extensions_of("image/*"), ["png", "jpg", "jpeg", "gif", "bmp", "webp"]);
        assert_eq!(extensions_of(".ogg,.wav"), ["ogg", "wav"]);
        assert!(extensions_of("").is_empty(), "no filter for an empty string");
        assert!(extensions_of("application/x-unknown,.").is_empty(), "what maps to nothing adds nothing");
        assert!(extensions_of(".p/ng").is_empty(), "a suffix with a separator in it is not an extension");
    }

    /// The native registry with a fake dialog in place of the system one.
    #[cfg(not(target_arch = "wasm32"))]
    mod native {
        use super::super::imp::{folder_files, start_dir_from, Dialog, Launch, Registry};
        use super::super::*;
        use std::cell::Cell;
        use std::path::{Path, PathBuf};
        use std::rc::Rc;

        fn temp(name: &str) -> PathBuf {
            let dir = std::env::temp_dir().join(format!("lr_file_pick_{name}_{}", crate::shared::task::process_id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            dir
        }

        /// A dialog that stays open for `open_for` polls (the first one is the poll that starts it), then answers.
        fn fake(open_for: u32, answer: Option<Vec<PathBuf>>) -> (Launch, Rc<Cell<u32>>) {
            let launched = Rc::new(Cell::new(0));
            let seen = launched.clone();
            let launch: Launch = Box::new(move || {
                seen.set(seen.get() + 1);
                let mut left = open_for;
                let mut answer = Some(answer);
                Dialog(Box::new(move || {
                    if left > 0 {
                        left -= 1;
                        None
                    } else {
                        answer.take()
                    }
                }))
            });
            (launch, launched)
        }

        #[test]
        fn the_dialog_opens_on_the_first_poll_and_stays_pending_while_it_is_open() {
            let dir = temp("pending");
            let file = dir.join("a.png");
            std::fs::write(&file, b"png").unwrap();
            let mut reg = Registry::default();
            let (launch, launched) = fake(3, Some(vec![file]));
            let id = reg.open_with(false, launch);
            assert_eq!(launched.get(), 0, "open() only queues: the frame that asked is drawn first");
            assert_eq!(poll_from(&mut reg, id, 100), Outcome::Pending);
            assert_eq!(launched.get(), 1);
            assert_eq!(poll_from(&mut reg, id, 100), Outcome::Pending);
            assert_eq!(poll_from(&mut reg, id, 100), Outcome::Pending);
            assert_eq!(launched.get(), 1, "the dialog is started once");
            assert_eq!(
                poll_from(&mut reg, id, 100),
                Outcome::Picked(vec![PickedFile { name: "a.png".into(), bytes: Ok(b"png".to_vec()) }])
            );
            assert_eq!(reg.state(id), State::Cancelled, "a finished request is released");
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[test]
        fn a_closed_dialog_changes_nothing() {
            let mut reg = Registry::default();
            let (launch, _) = fake(0, None);
            let id = reg.open_with(false, launch);
            assert_eq!(poll_from(&mut reg, id, 100), Outcome::Cancelled);
            let (launch, _) = fake(0, Some(Vec::new()));
            let id = reg.open_with(false, launch);
            assert_eq!(poll_from(&mut reg, id, 100), Outcome::Cancelled, "a dialog that chose no file is cancelled too");
            assert_eq!(poll_from(&mut reg, 999, 100), Outcome::Cancelled, "an unknown request is cancelled");
        }

        #[test]
        fn a_file_over_the_cap_is_refused_without_being_read_and_the_others_still_come() {
            let dir = temp("cap");
            let (big, small) = (dir.join("big.png"), dir.join("small.png"));
            std::fs::write(&big, vec![7u8; 50]).unwrap();
            std::fs::write(&small, vec![1u8; 10]).unwrap();
            let mut reg = Registry::default();
            let (launch, _) = fake(0, Some(vec![big, small]));
            let id = reg.open_with(false, launch);
            let Outcome::Picked(files) = poll_from(&mut reg, id, 20) else { panic!("files were chosen") };
            assert_eq!(files.len(), 2);
            assert_eq!(files[0].bytes, Err("the file is too big".to_string()));
            assert_eq!(files[1].bytes, Ok(vec![1u8; 10]));
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[test]
        fn a_folder_gives_its_files_named_from_the_folder() {
            let dir = temp("folder").join("skins");
            std::fs::create_dir_all(dir.join("old")).unwrap();
            std::fs::write(dir.join("b.png"), b"b").unwrap();
            std::fs::write(dir.join("old").join("a.png"), b"a").unwrap();
            let names: Vec<String> = folder_files(&dir).into_iter().map(|c| c.name).collect();
            assert_eq!(names, ["skins/b.png", "skins/old/a.png"]);
            let mut reg = Registry::default();
            let (launch, _) = fake(0, Some(vec![dir.clone()]));
            let id = reg.open_with(true, launch);
            let Outcome::Picked(files) = poll_from(&mut reg, id, 100) else { panic!("a folder was chosen") };
            assert_eq!(files.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), ["skins/b.png", "skins/old/a.png"]);
            let _ = std::fs::remove_dir_all(dir.parent().unwrap());
        }

        #[test]
        fn a_request_cancelled_while_its_dialog_is_open_ignores_the_late_answer() {
            let dir = temp("cancel_open");
            let file = dir.join("late.png");
            std::fs::write(&file, b"late").unwrap();
            let mut reg = Registry::default();
            let (launch, launched) = fake(2, Some(vec![file]));
            let id = reg.open_with(false, launch);
            assert_eq!(poll_from(&mut reg, id, 100), Outcome::Pending, "the dialog opens on the first poll");
            assert_eq!(reg.state(id), State::Pending, "still open");
            reg.release(id);
            assert_eq!(reg.state(id), State::Cancelled, "a released request reads as cancelled at once");
            assert_eq!(poll_from(&mut reg, id, 100), Outcome::Cancelled, "the dialog's later answer is never taken");
            assert_eq!(launched.get(), 1, "the dialog was started once and not again");
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[test]
        fn a_missing_start_folder_gives_no_start_and_does_not_panic() {
            let missing = PathBuf::from("/no/such/lr/home");
            assert_eq!(start_dir_from(Some(missing.clone()), &|d| d.is_dir()), None, "a home folder that is not there");
            assert_eq!(start_dir_from(None, &|d| d.is_dir()), None, "no home variable at all");
        }

        #[test]
        fn a_released_request_is_ignored() {
            let mut reg = Registry::default();
            let (launch, launched) = fake(5, None);
            let id = reg.open_with(false, launch);
            reg.release(id);
            assert_eq!(reg.state(id), State::Cancelled);
            assert_eq!(launched.get(), 0, "a request let go of before its first poll never opens a dialog");
        }

        #[test]
        fn the_dialog_starts_in_pictures_then_desktop_then_home() {
            let home = PathBuf::from("/home/kt");
            let has = |dirs: &'static [&'static str]| {
                move |d: &Path| d == Path::new("/home/kt") || dirs.iter().any(|x| d == Path::new("/home/kt").join(x))
            };
            assert_eq!(start_dir_from(Some(home.clone()), &has(&["Pictures", "Desktop"])), Some(home.join("Pictures")));
            assert_eq!(start_dir_from(Some(home.clone()), &has(&["Desktop"])), Some(home.join("Desktop")));
            assert_eq!(start_dir_from(Some(home.clone()), &has(&[])), Some(home.clone()));
            assert_eq!(start_dir_from(Some(home.clone()), &|_| false), None, "nothing there");
            assert_eq!(start_dir_from(None, &|_| true), None, "no home folder");
            assert_eq!(
                start_dir_from(Some(home.clone()), &|d| d == home.join("OneDrive").join("Pictures")),
                Some(home.join("OneDrive").join("Pictures")),
                "a Pictures folder that OneDrive moved"
            );
        }
    }
}
