mod logger;

use std::mem;

use log::{debug, error, info, Level};
use tokio::{
    signal::windows::ctrl_c,
    sync::oneshot::{channel, Sender},
};
use windows::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    System::LibraryLoader::GetModuleHandleA,
    UI::{
        Input::KeyboardAndMouse::GetKeyNameTextA,
        WindowsAndMessaging::{
            CallNextHookEx, DispatchMessageA, GetForegroundWindow, GetMessageA, GetWindowTextA,
            GetWindowTextLengthA, SetWindowsHookExA, TranslateMessage, UnhookWindowsHookEx, HHOOK,
            KBDLLHOOKSTRUCT, MSG, WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
        },
    },
};

struct Container<'a>(&'a mut (dyn RecordWriter + 'a));

static mut CONTAINER: *mut Container<'static> = 0 as *mut _;

#[derive(Debug)]
struct Record {
    key: KBDLLHOOKSTRUCT,
    hwnd: HWND,
}

impl Record {
    fn new(key: KBDLLHOOKSTRUCT, hwnd: HWND) -> Self {
        Record { key, hwnd }
    }

    fn time(&self) -> u32 {
        self.key.time
    }

    fn window_title(&self) -> String {
        let len: usize = unsafe { GetWindowTextLengthA(self.hwnd) } as usize;
        debug!(">>>>>>>>>>>>>>>>>>>>>>>>>>> {}", len);
        let mut lp_string = vec![0; len + 1];
        debug!(">>>>>>>>>>>>>>>>>>>>>>>>>>> {}", lp_string.len());
        if unsafe { GetWindowTextA(self.hwnd, &mut lp_string) } > 0 {
            if let Ok(title) = String::from_utf8(lp_string) {
                return title;
            }
        } else {
            debug!("cannot get window text...");
        }
        String::from("unknown")
    }

    fn key_text(&self) -> String {
        let mut lp_string: [u8; 16] = [0; 16];

        let key_text;
        if unsafe { GetKeyNameTextA((self.key.scanCode << 16) as i32, &mut lp_string) } <= 0 {
            debug!("failed to get key text: {:#?}", self.key);
            key_text = String::from("unknown").into();
        } else {
            match String::from_utf8(lp_string.to_vec()) {
                Ok(s) => {
                    key_text = s.clone().trim_end_matches('\0').to_string();
                }
                Err(e) => {
                    error!("unable to get key text from [{:#?}]: {}", lp_string, e);
                    key_text = String::from("unknown").into();
                }
            };
        }
        key_text
    }
}

trait RecordWriter {
    fn write(&mut self, record: Record);
}

struct ConsoleWriter {}

impl RecordWriter for ConsoleWriter {
    fn write(&mut self, record: Record) {
        info!(
            "{} - the key [{}] has been triggered, the window title is \"{}\"",
            record.time(),
            record.key_text(),
            record.window_title()
        );
    }
}

impl ConsoleWriter {
    fn new() -> Self {
        ConsoleWriter {}
    }
}

async fn uninstall_keyboard_hook(h_hook: HHOOK) -> Result<(), Box<dyn std::error::Error>> {
    debug!("exit the program and uninstall the hook.");
    unsafe { UnhookWindowsHookEx(h_hook) };
    Ok(())
}

async fn install_keyboard_hook(sender: Sender<HHOOK>) {
    let result = unsafe {
        GetModuleHandleA(None).and_then(|app| {
            SetWindowsHookExA(WH_KEYBOARD_LL, Some(low_level_keyboard_proc), app, 0)
        })
    };

    match result {
        Ok(h_hook) => {
            debug!("successfully set windows hook.");
            if let Ok(()) = sender.send(h_hook) {
                debug!("successfully send h_hook to channel...");
            }
            let mut msg = MSG::default();
            let result = unsafe { GetMessageA(&mut msg, None, 0, 0) };
            while result.0 > 0 {
                unsafe {
                    TranslateMessage(&msg);
                    DispatchMessageA(&msg);
                };
            }
        }
        Err(e) => error!("failed to set hook: {}", e),
    };
}

unsafe extern "system" fn low_level_keyboard_proc(
    n_code: i32,
    w_param: WPARAM,
    l_param: LPARAM,
) -> LRESULT {
    debug!("keyboard hook proc has been triggered...");
    let event_type = w_param.0 as u32;
    let key = l_param.0 as *const KBDLLHOOKSTRUCT;
    match event_type {
        WM_SYSKEYDOWN | WM_KEYDOWN => {
            debug!(
                "the key down event has been triggered: {:#?}",
                (*key).vkCode
            );
            let c = unsafe { &mut *CONTAINER };
            c.0.write(Record::new(*key, GetForegroundWindow()));
        }
        WM_SYSKEYUP | WM_KEYUP => {
            debug!("the key up event has been triggered: {:#?}", (*key).vkCode);
        }
        _ => {
            debug!(
                "unknown event type, w_param: {:#?}, l_param: {:#?}",
                w_param, l_param
            );
        }
    };
    CallNextHookEx(None, n_code, w_param, l_param)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    logger::init(if cfg!(debug_assertions) {
        Level::Debug
    } else {
        Level::Info
    })?;
    debug!("program has been started...");
    let (hook_sender, hook_receiver) = channel();
    let mut console_writer = ConsoleWriter::new();
    let c = Container(&mut console_writer);
    unsafe {
        CONTAINER = mem::transmute(&c);
    }

    tokio::spawn(async {
        install_keyboard_hook(hook_sender).await;
    });
    let h_hook = hook_receiver.await?;
    let mut signal = ctrl_c()?;
    signal.recv().await;
    debug!("ctrl_c has been pressed...");
    let _ = uninstall_keyboard_hook(h_hook).await;
    std::process::exit(0);
}
