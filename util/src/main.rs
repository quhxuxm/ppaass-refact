use anyhow::Ok;
use anyhow::Result;

use common::{generate_agent_key_pairs, generate_proxy_key_pairs};

fn main() -> Result<()> {
    println!("Begin to generate rsa key");
    generate_agent_key_pairs("rsa", "user3")?;
    generate_proxy_key_pairs("rsa", "user3")?;
    generate_agent_key_pairs("rsa", "user4")?;
    generate_proxy_key_pairs("rsa", "user4")?;
    Ok(())
}
