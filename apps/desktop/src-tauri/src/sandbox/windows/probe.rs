//! Standalone fixture compiled with rustc by the ignored Windows test.

use std::{
    fs::{self, File},
    io::{self, Read},
    os::windows::io::FromRawHandle,
    path::PathBuf,
};

fn error_code<T>(result: io::Result<T>) -> i32 {
    result
        .err()
        .and_then(|error| error.raw_os_error())
        .unwrap_or(0)
}

#[link(name = "ws2_32")]
unsafe extern "system" {
    fn WSAStartup(version: u16, data: *mut u8) -> i32;
    fn WSAGetLastError() -> i32;
    fn socket(family: i32, kind: i32, protocol: i32) -> usize;
    fn ioctlsocket(socket: usize, command: i32, argument: *mut u32) -> i32;
    fn connect(socket: usize, address: *const u8, length: i32) -> i32;
    fn select(
        count: i32,
        read: *mut u8,
        write: *mut Fdset,
        except: *mut u8,
        timeout: *const [i32; 2],
    ) -> i32;
    fn closesocket(socket: usize) -> i32;
}

#[repr(C)]
struct Fdset {
    count: u32,
    sockets: [usize; 64],
}

/// Raw Winsock calls: std panics when Winsock cannot start, which is itself a denial.
fn tcp(address: &str) -> String {
    let (host, port) = address.rsplit_once(':').unwrap();
    let mut octets = [0u8; 4];
    for (slot, part) in octets.iter_mut().zip(host.split('.')) {
        *slot = part.parse().unwrap();
    }
    let port: u16 = port.parse().unwrap();
    let mut data = [0u8; 512];
    let started = unsafe { WSAStartup(0x0202, data.as_mut_ptr()) };
    if started != 0 {
        return format!("denied: WSAStartup {started}");
    }
    let socket = unsafe { socket(2, 1, 6) };
    if socket == usize::MAX {
        return format!("denied: socket {}", unsafe { WSAGetLastError() });
    }
    let mut nonblocking = 1u32;
    unsafe { ioctlsocket(socket, 0x8004667eu32 as i32, &mut nonblocking) };
    let mut sockaddr = [0u8; 16];
    sockaddr[..2].copy_from_slice(&2u16.to_le_bytes());
    sockaddr[2..4].copy_from_slice(&port.to_be_bytes());
    sockaddr[4..8].copy_from_slice(&octets);
    let result = if unsafe { connect(socket, sockaddr.as_ptr(), 16) } == 0 {
        "connected".to_string()
    } else {
        let code = unsafe { WSAGetLastError() };
        if code != 10035 {
            format!("denied: connect {code}")
        } else {
            let mut write = Fdset {
                count: 1,
                sockets: [0; 64],
            };
            write.sockets[0] = socket;
            let ready = unsafe {
                select(
                    0,
                    std::ptr::null_mut(),
                    &mut write,
                    std::ptr::null_mut(),
                    &[2, 0],
                )
            };
            if ready == 1 {
                "connected".to_string()
            } else {
                "denied: no connection within 2 s".to_string()
            }
        }
    };
    unsafe { closesocket(socket) };
    result
}

fn main() -> io::Result<()> {
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    if args[0] == "idle" {
        loop {
            std::thread::park();
        }
    }
    if args[0] == "net" {
        println!("{}", tcp(args[1].to_str().unwrap()));
        return Ok(());
    }
    println!("allowed read: {}", fs::read_to_string(&args[0])?);
    println!("private read: {}", error_code(fs::read(&args[1])));
    println!(
        "outside write: {}",
        error_code(fs::write(&args[2], b"blocked"))
    );
    let temp = PathBuf::from(std::env::var_os("TEMP").unwrap());
    assert_eq!(Some(temp.as_os_str()), std::env::var_os("TMP").as_deref());
    let cwd = std::env::current_dir()?;
    assert_eq!(cwd, PathBuf::from(&args[6]));
    fs::write(cwd.join("created.txt"), b"scratch")?;
    println!("scratch write: ok");
    for (label, address) in [
        ("loopback TCP", args[3].to_str().unwrap()),
        ("external TCP", "1.1.1.1:443"),
    ] {
        println!("{label}: {}", tcp(address));
    }
    println!("{}", args[5].to_string_lossy());
    eprintln!("stderr probe");
    io::copy(&mut io::stdin(), &mut io::stdout())?;
    println!();
    ambient_handle(args[4].to_str().unwrap().parse().unwrap());
    Ok(())
}

#[repr(C)]
struct ExceptionPointers {
    record: *const u32,
    context: *const std::ffi::c_void,
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetHandleInformation(handle: *mut std::ffi::c_void, flags: *mut u32) -> i32;
    fn AddVectoredExceptionHandler(
        first: u32,
        handler: unsafe extern "system" fn(*const ExceptionPointers) -> i32,
    ) -> *mut std::ffi::c_void;
    fn ExitProcess(code: u32) -> !;
}

/// Strict handle checks make a value that is not a handle in this process raise
/// STATUS_INVALID_HANDLE instead of returning an error. That is the expected result.
unsafe extern "system" fn invalid_handle(pointers: *const ExceptionPointers) -> i32 {
    if unsafe { *(&*pointers).record } == 0xC000_0008 {
        println!("ambient handle: absent");
        unsafe { ExitProcess(0) };
    }
    0
}

fn ambient_handle(handle: usize) {
    unsafe { AddVectoredExceptionHandler(1, invalid_handle) };
    let mut flags = 0;
    // The handle value can be reused for an unrelated handle in the new process.
    let inherited =
        unsafe { GetHandleInformation(handle as _, &mut flags) } != 0 && flags & 1 != 0 && {
            let mut file =
                std::mem::ManuallyDrop::new(unsafe { File::from_raw_handle(handle as _) });
            let mut bytes = [0; 7];
            file.read_exact(&mut bytes).is_ok() && &bytes == b"private"
        };
    println!(
        "ambient handle: {}",
        if inherited { "present" } else { "absent" }
    );
}
