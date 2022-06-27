use anyhow::anyhow;
use anyhow::Result;

use clap::Parser;
use common::{generate_agent_key_pairs, generate_proxy_key_pairs};

#[derive(Parser, Debug)]
#[clap(name="ppaass-util", author="Qu Hao", version="1.0", about, long_about = None)]
/// The tool to generate rsa file for different users.
struct UtilArguments {
    /// Generate rsa file for user name
    #[clap(short = 'u', long, value_parser)]
    pub user_name: Option<String>,
    /// The base directory to store rsa files
    #[clap(short = 'b', long, value_parser)]
    pub base_dir: Option<String>,
}

fn main() -> Result<()> {
    println!("Begin to generate rsa key");
    let util_arguments = UtilArguments::parse();
    let user_name = util_arguments.user_name.ok_or(anyhow!("Fail to parse user name from arguments"))?;
    let base_dir = util_arguments.base_dir.unwrap_or("rsa".to_string());
    generate_agent_key_pairs(&base_dir, &user_name)?;
    generate_proxy_key_pairs(&base_dir, &user_name)?;
    println!("Success to generate rsa files for user: {} in {}", &user_name, &base_dir);
    Ok(())
}
