use std::env;
use std::error::Error;
use std::fs;

fn main() {
    if let Err(e) = try_main() {
        eprintln!("{}", e);
        std::process::exit(1);
    }
}

fn try_main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = env::args().collect();
    let verbose = args.contains(&"-v".to_string()) || args.contains(&"--verbose".to_string());
    
    // Validate args
    if args.len() < 2 || (args.len() == 2 && verbose) {
        return Err("Usage: <program> <firmware.bin> [-v]".into());
    }
    
    let firmware_file = &args[1];
    println!("[*] Firmware: {}", firmware_file);
    if verbose { println!("[*] Verbose mode enabled"); }

    // USB constants
    const VID: u16 = 0x05a9;
    const PID_BOOT: u16 = 0x0580;
    const PID_FW: u16 = 0x058c;
    const IFACE: u8 = 0;
    const PKT_SIZE: usize = 512;
    const REQ_TYPE: u8 = 0x40;
    const MIN_SIZE: usize = 64 * 1024;

    if verbose {
        println!("[*] USB Configuration - VID: 0x{:04x}, PID (bootloader): 0x{:04x}, PID (firmware): 0x{:04x}", 
                 VID, PID_BOOT, PID_FW);
        println!("[*] Interface: {}, Max packet size: {} bytes, Min firmware size: {} bytes", 
                 IFACE, PKT_SIZE, MIN_SIZE);
    }

    // Find device in bootloader mode
    println!("[*] Searching for device...");
    let dev = match rusb::open_device_with_vid_pid(VID, PID_BOOT) {
        Some(h) => { 
            println!("[✓] Found bootloader mode"); 
            h 
        }
        None => {
            if verbose { println!("[*] Bootloader mode not found, checking camera mode..."); }
            // Check if already in camera mode
            if rusb::open_device_with_vid_pid(VID, PID_FW).is_some() {
                println!("[✓] Already in camera mode");
                return Ok(());
            }
            return Err("Camera not found (VID:PID 05a9:0580 or 05a9:058c)".into());
        }
    };

    // Setup USB interface
    if verbose { println!("[*] Checking kernel driver status..."); }
    if let Ok(true) = dev.kernel_driver_active(IFACE) {
        if verbose { println!("[!] Kernel driver is active, detaching..."); }
        let _ = dev.detach_kernel_driver(IFACE);
        if verbose { println!("[✓] Kernel driver detached"); }
    } else if verbose {
        println!("[✓] Kernel driver not active");
    }
    
    if verbose { println!("[*] Claiming interface {}...", IFACE); }
    dev.claim_interface(IFACE)?;
    if verbose { println!("[✓] Interface claimed"); }
    println!("[✓] USB ready");

    // Read firmware file
    println!("[*] Reading firmware...");
    let fw = fs::read(firmware_file)?;
    let fw_len = fw.len();
    println!("[*] Size: {} bytes ({:.2} KB)", fw_len, fw_len as f64 / 1024.0);
    
    if fw_len < MIN_SIZE {
        return Err(format!("File too small: {} < {}", fw_len, MIN_SIZE).into());
    }
    if verbose { println!("[✓] Firmware size is valid"); }

    // Upload firmware in packets
    println!("[*] Uploading...");
    if verbose {
        println!("[*] Total packets to send: {}", (fw_len + PKT_SIZE - 1) / PKT_SIZE);
    }
    
    let mut offset = 0;
    let mut w_value: u16 = 0;
    let mut pkt_num = 0;

    while offset < fw_len {
        let size = std::cmp::min(fw_len - offset, PKT_SIZE);
        let idx = if offset < (u16::MAX as usize) { 0x14 } else { 0x15 };

        if verbose {
            println!("[*] Packet {}: {} bytes [offset: {} - {}], wValue=0x{:04x}, wIndex=0x{:04x}",
                     pkt_num, size, offset, offset + size, w_value, idx);
        }

        match dev.write_control(REQ_TYPE, 0x0, w_value, idx, &fw[offset..offset + size], std::time::Duration::from_secs(5)) {
            Ok(bytes) => {
                if verbose {
                    println!("[✓] Packet {} transferred {} bytes", pkt_num, bytes);
                }
            }
            Err(e) => {
                println!("[✗] Packet {} failed: {:?}", pkt_num, e);
                return Err(format!("Failed to send packet {}: {:?}", pkt_num, e).into());
            }
        }
        
        w_value = w_value.wrapping_add(size as u16);
        offset += size;
        pkt_num += 1;
        std::thread::sleep(std::time::Duration::from_millis(1));
    }

    println!("[✓] {} packets sent", pkt_num);

    // Send footer packet
    println!("[*] Finalizing...");
    if verbose { println!("[*] Sending footer packet (0x5B)..."); }
    match dev.write_control(REQ_TYPE, 0x0, 0x2200, 0x8018, &[0x5B], std::time::Duration::from_secs(5)) {
        Ok(bytes) => {
            if verbose { println!("[✓] Footer packet sent ({} bytes)", bytes); }
            println!("[✓] Done!");
        }
        Err(rusb::Error::NoDevice) => {
            if verbose { println!("[*] Device disconnected during finalization"); }
            println!("[✓] Device restarting...");
        }
        Err(e) => {
            println!("[✗] Footer failed: {:?}", e);
            return Err(format!("Footer failed: {:?}", e).into());
        }
    }

    Ok(())
}
