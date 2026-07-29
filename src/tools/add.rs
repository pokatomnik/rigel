use std::convert::Infallible;

use rig::tool::{Tool, ToolContext};
use serde::Deserialize;

#[derive(Deserialize)]
pub(crate) struct AddArgs {
    x: i64,
    y: i64,
}

pub(crate) struct Adder;

impl Tool for Adder {
    const NAME: &'static str = "add";
    type Args = AddArgs;
    type Output = i64;
    type Error = Infallible;

    fn description(&self) -> String {
        "Add x and y together".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "x": {
                    "type": "integer",
                    "description": "The first number to add"
                },
                "y": {
                    "type": "integer",
                    "description": "The second number to add"
                }
            },
            "required": ["x", "y"]
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        Ok(args.x + args.y)
    }
}
