use std::fs;

fn main() {
    load_env();
    linker_be_nice();
    // make sure linkall.x is the last linker script (otherwise might cause problems with flip-link)
    println!("cargo:rustc-link-arg=-Tlinkall.x");
}

/// Reads variables from a `.env` file in the workspace root (if present) and
/// re-exports them as Cargo environment variables.
///
/// Variables loaded:
///   - `WIFI_SSID`               → `env!("WIFI_SSID")`
///   - `WIFI_PASSWORD`           → `env!("WIFI_PASSWORD")`
///   - `HOST_IP`                 → `env!("HOST_IP_0")` .. `env!("HOST_IP_3")` (octets)
///   - `PORT`               → `env!("PORT")` (validated u16)
///   - `WASHER_GPIO`             → `env!("WASHER_GPIO")` (validated u8)
///   - `WASHER_TOPIC`            → `env!("WASHER_TOPIC")`
///   - `WASHER_IDLE_TIMEOUT_SECS`→ `env!("WASHER_IDLE_TIMEOUT_SECS")` (validated u64)
///   - `DRYER_GPIO`              → `env!("DRYER_GPIO")` (validated u8)
///   - `DRYER_TOPIC`             → `env!("DRYER_TOPIC")`
///   - `DRYER_IDLE_TIMEOUT_SECS` → `env!("DRYER_IDLE_TIMEOUT_SECS")` (validated u64)
///
/// Precedence: a value already present in the *process* environment (e.g. set
/// by CI or exported in the shell) always wins over the `.env` file, so you
/// can override without touching the file.
fn load_env() {
    // Tell Cargo to re-run this script whenever .env changes.
    println!("cargo:rerun-if-changed=.env");
    // Re-run if any watched variable changes in the shell environment.
    for var in [
        "WIFI_SSID",
        "WIFI_PASSWORD",
        "HOST_IP",
        "PORT",
        "WASHER_GPIO",
        "WASHER_TOPIC",
        "WASHER_IDLE_TIMEOUT_SECS",
        "DRYER_GPIO",
        "DRYER_TOPIC",
        "DRYER_IDLE_TIMEOUT_SECS",
    ] {
        println!("cargo:rerun-if-env-changed={var}");
    }

    // Parse .env file into a simple key→value map (best-effort; missing file
    // is not an error — the vars might come from the shell environment).
    let mut file_vars = std::collections::HashMap::new();
    if let Ok(contents) = fs::read_to_string(".env") {
        for line in contents.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((key, val)) = line.split_once('=') {
                file_vars.insert(key.trim().to_string(), val.trim().to_string());
            }
        }
    }

    // Helper: resolve a variable from the environment or .env file.
    let resolve = |var: &str| -> String {
        std::env::var(var)
            .ok()
            .or_else(|| file_vars.get(var).cloned())
            .unwrap_or_else(|| {
                panic!(
                    "\n\n\
                    ❌  `{var}` is not set.\n\
                    \n\
                    Add it to a `.env` file in the project root:\n\
                    \n\
                    {var}=your_value\n\
                    \n\
                    …or export it in your shell before running `cargo build`.\n"
                )
            })
    };

    // -- String vars ----------------------------------------------------------
    for var in ["WIFI_SSID", "WIFI_PASSWORD", "WASHER_TOPIC", "DRYER_TOPIC"] {
        println!("cargo:rustc-env={var}={}", resolve(var));
    }

    // -- HOST_IP → individual octets ------------------------------------------
    let host_ip = resolve("HOST_IP");
    let octets: Vec<u8> = host_ip
        .trim()
        .split('.')
        .map(|o| {
            o.parse::<u8>()
                .unwrap_or_else(|_| panic!("❌  HOST_IP `{host_ip}` is not a valid IPv4 address"))
        })
        .collect();
    assert!(
        octets.len() == 4,
        "❌  HOST_IP `{host_ip}` must have exactly 4 octets"
    );
    for (i, octet) in octets.iter().enumerate() {
        println!("cargo:rustc-env=HOST_IP_{i}={octet}");
    }

    // -- PORT → validated u16 --------------------------------------------
    let port_str = resolve("PORT");
    let port: u16 = port_str
        .trim()
        .parse()
        .unwrap_or_else(|_| panic!("❌  PORT `{port_str}` must be a valid u16 (0–65535)"));
    println!("cargo:rustc-env=PORT={port}");

    // -- Appliance GPIO pins → validated u8 -----------------------------------
    for var in ["WASHER_GPIO", "DRYER_GPIO"] {
        let val_str = resolve(var);
        let val: u8 = val_str.trim().parse().unwrap_or_else(|_| {
            panic!("❌  `{var}` `{val_str}` must be a valid GPIO pin number (0–39)")
        });
        println!("cargo:rustc-env={var}={val}");
    }

    // -- Appliance idle timeouts → validated u64 ------------------------------
    for var in ["WASHER_IDLE_TIMEOUT_SECS", "DRYER_IDLE_TIMEOUT_SECS"] {
        let val_str = resolve(var);
        let val: u64 = val_str.trim().parse().unwrap_or_else(|_| {
            panic!("❌  `{var}` `{val_str}` must be a valid number of seconds")
        });
        println!("cargo:rustc-env={var}={val}");
    }
}

fn linker_be_nice() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 {
        let kind = &args[1];
        let what = &args[2];

        match kind.as_str() {
            "undefined-symbol" => match what.as_str() {
                what if what.starts_with("_defmt_") => {
                    eprintln!();
                    eprintln!(
                        "💡 `defmt` not found - make sure `defmt.x` is added as a linker script and you have included `use defmt_rtt as _;`"
                    );
                    eprintln!();
                }
                "_stack_start" => {
                    eprintln!();
                    eprintln!("💡 Is the linker script `linkall.x` missing?");
                    eprintln!();
                }
                what if what.starts_with("esp_rtos_") => {
                    eprintln!();
                    eprintln!(
                        "💡 `esp-radio` has no scheduler enabled. Make sure you have initialized `esp-rtos` or provided an external scheduler."
                    );
                    eprintln!();
                }
                "embedded_test_linker_file_not_added_to_rustflags" => {
                    eprintln!();
                    eprintln!(
                        "💡 `embedded-test` not found - make sure `embedded-test.x` is added as a linker script for tests"
                    );
                    eprintln!();
                }
                "free"
                | "malloc"
                | "calloc"
                | "get_free_internal_heap_size"
                | "malloc_internal"
                | "realloc_internal"
                | "calloc_internal"
                | "free_internal" => {
                    eprintln!();
                    eprintln!(
                        "💡 Did you forget the `esp-alloc` dependency or didn't enable the `compat` feature on it?"
                    );
                    eprintln!();
                }
                _ => (),
            },
            // we don't have anything helpful for "missing-lib" yet
            _ => {
                std::process::exit(1);
            }
        }

        std::process::exit(0);
    }

    println!(
        "cargo:rustc-link-arg=-Wl,--error-handling-script={}",
        std::env::current_exe().unwrap().display()
    );
}
