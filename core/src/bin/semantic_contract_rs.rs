use local_os_agent::semantic_contract;
use std::io::Read;

fn parse_arg(flag: &str) -> Option<String> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == flag {
            return args.next();
        }
        if let Some(rest) = arg.strip_prefix(&(flag.to_string() + "=")) {
            return Some(rest.to_string());
        }
    }
    None
}

fn main() {
    let mode = parse_arg("--mode").unwrap_or_else(|| "json".to_string());
    let request = parse_arg("--request").unwrap_or_else(|| {
        let mut input = String::new();
        let _ = std::io::stdin().read_to_string(&mut input);
        input
    });

    let contract = semantic_contract::parse_contract(&request);
    match mode.as_str() {
        "tokens" => {
            for token in contract.tokens {
                println!("{token}");
            }
        }
        "recipients" => {
            for recipient in contract.recipients {
                println!("{recipient}");
            }
        }
        _ => {
            println!(
                "{}",
                serde_json::to_string(&contract).unwrap_or_else(|_| "{}".to_string())
            );
        }
    }
}
