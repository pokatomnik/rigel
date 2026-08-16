use crate::controllers::chat_controller::ChatController;
use clap::Subcommand;

#[derive(Subcommand, Clone)]
pub(crate) enum Commands {
    Chat(ChatController),
}
