use clap::Parser;

use crate::controllers::index_controller::IndexController;

#[derive(Parser)]
#[command(name = "Rigel")]
#[command(about = "Cozy LLM agent")]
#[command(version)]
pub(crate) struct Cli {
    #[command(flatten)]
    pub index: IndexController,
}
