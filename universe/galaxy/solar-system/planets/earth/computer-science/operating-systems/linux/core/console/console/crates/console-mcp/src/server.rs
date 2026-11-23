use crate::evaluation::Evaluator;
use console_protocol::{UseAnsiColoring, engine::EngineState};
use rmcp::{
    RoleServer, ServerHandler,
    handler::server::{tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, Implementation, ServerCapabilities, ServerConfig},
    service::RequestContext,
    tool, tool_handler, tool_router,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub struct NushellMcpServer {
    #[allow(dead_code)]
    tool_router: ToolRouter<Self>,
    evaluator: Evaluator,
}

#[tool_router]
impl NushellMcpServer {
    pub fn new(mut engine_state: EngineState) -> Self {
        // Configure the engine state for MCP
        let mut config = engine_state.get_config().as_ref().clone();
        config.use_ansi_coloring = UseAnsiColoring::False;
        config.color_config.clear();
        engine_state.set_config(config);
        NushellMcpServer {
            tool_router: Self::tool_router(),
            evaluator: Evaluator::new(engine_state),
        }
    }

    #[tool(
        description = r#"List available Nushell native commands, one per line as `<call signature> - <description>`.
Without `find`, every command in scope is returned (hundreds of lines). With `find`, only commands whose name, description, or search terms contain that string (case-insensitive) are returned; when nothing matches, the tool returns an error.
Use command_help for one command's flags and parameters."#
    )]
    async fn list_commands(
        &self,
        _ctx: RequestContext<RoleServer>,
        Parameters(ListCommandsRequest { find }): Parameters<ListCommandsRequest>,
    ) -> Result<String, String> {
        self.evaluator.list_available_commands(find).await
    }

    #[tool(
        description = "Get help for one Nushell command: its usage line, description, details, search terms, flags, and positional parameters. Pass the full name as you would call it, including subcommands (for example `str replace`). It covers commands defined in this session (built-ins, loaded plugin commands, and custom commands from earlier evaluations), not external programs; use the list_commands tool to see which names exist. The output does not include input/output types or examples; evaluate `help <name>` for those."
    )]
    async fn command_help(
        &self,
        _ctx: RequestContext<RoleServer>,
        Parameters(CommandNameRequest { name }): Parameters<CommandNameRequest>,
    ) -> Result<String, String> {
        self.evaluator.command_help(&name).await
    }

    #[doc = include_str!("evaluate_tool.md")]
    #[tool]
    async fn evaluate(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(NuSourceRequest { input }): Parameters<NuSourceRequest>,
    ) -> CallToolResult {
        self.evaluator.eval_async(&input, ctx.ct).await
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
struct ListCommandsRequest {
    #[schemars(description = "string to find in command names, descriptions, and search term")]
    find: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
struct CommandNameRequest {
    #[schemars(description = "The name of the command")]
    name: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
struct NuSourceRequest {
    #[schemars(description = "The Nushell source code to evaluate")]
    input: String,
}

#[tool_handler]
impl ServerHandler for NushellMcpServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(
                Implementation::new("console-mcp-server", env!("CARGO_PKG_VERSION"))
                    .with_title("Nushell MCP Server")
                    .with_website_url("https://www.nushell.sh"),
            )
            .with_instructions(include_str!("instructions.md"))
    }
}

