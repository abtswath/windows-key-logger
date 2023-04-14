mod logger;

use std::{mem, sync::RwLock};

use log::{debug, error, info, Level};
use tokio::{
    signal::windows::ctrl_c,
    sync::oneshot::{channel, Sender},
};
use windows::Win32::{
    Foundation::{LPARAM, LRESULT, WPARAM},
    System::LibraryLoader::GetModuleHandleA,
    UI::{
        Input::KeyboardAndMouse::GetKeyNameTextA,
        WindowsAndMessaging::{
            CallNextHookEx, DispatchMessageA, GetMessageA, SetWindowsHookExA, TranslateMessage,
            UnhookWindowsHookEx, HHOOK, KBDLLHOOKSTRUCT, MSG, WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP,
            WM_SYSKEYDOWN, WM_SYSKEYUP,
        },
    },
};

struct Container<'a>(&'a mut (dyn RecordWriter + 'a));

static mut CONTAINER: *mut Container<'static> = 0 as *mut _;

static KEYS: RwLock<Vec<Key>> = RwLock::new(vec![]);

struct Key {
    key: KBDLLHOOKSTRUCT,
    released: bool,
}

impl Key {
    fn new(key: KBDLLHOOKSTRUCT) -> Self {
        Key {
            key,
            released: false,
        }
    }
}

#[derive(Debug)]
struct Record {
    keys: Vec<KBDLLHOOKSTRUCT>,
    key_text: String,
}

impl Record {
    fn new(keys: Vec<KBDLLHOOKSTRUCT>, key_text: String) -> Self {
        Record { keys, key_text }
    }
}

trait RecordWriter {
    fn write(&mut self, record: Record);
}

struct ConsoleWriter {}

impl RecordWriter for ConsoleWriter {
    fn write(&mut self, record: Record) {
        debug!("the record keys: {:#?}", record.keys);
        info!("the key [{}] has been triggered", record.key_text);
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

fn get_key_text(key: KBDLLHOOKSTRUCT) -> String {
    let mut lp_string: [u8; 16] = [0; 16];
    let key_text;
    if unsafe { GetKeyNameTextA((key.scanCode << 16) as i32, &mut lp_string) } <= 0 {
        debug!("failed to get key text:{:#?}", key);
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
            match KEYS.write() {
                Ok(mut keys) => {
                    debug!("new key has been pressed...");
                    (*keys).push(Key::new(*key));
                }
                Err(e) => {
                    error!("cannot get keys (key_down): {}", e);
                }
            }
        }
        WM_SYSKEYUP | WM_KEYUP => {
            debug!("the key up event has been triggered: {:#?}", (*key).vkCode);
            match KEYS.write() {
                Ok(mut keys) => {
                    debug!("new key has been released...");
                    let mut i = (*keys).len();
                    while i > 0 {
                        i -= 1;
                        if !(*keys)[i].released {
                            (*keys)[i].released = true;
                            break;
                        }
                    }
                    if i == 0 {
                        debug!("all keys has been released...");
                        let mut record_text = vec![];
                        let mut record_keys = vec![];
                        keys.iter().for_each(|key| {
                            record_keys.push(key.key);
                            record_text.push(get_key_text(key.key));
                        });
                        (*keys).clear();

                        let c = unsafe { &mut *CONTAINER };
                        c.0.write(Record::new(record_keys, record_text.join(" + ")));
                    }
                }
                Err(e) => {
                    error!("cannot get keys (key_up): {}", e);
                }
            }
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
