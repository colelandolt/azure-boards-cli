use azure_boards_cli::cli::{Cli, Commands};
use azure_boards_cli::commands;
use azure_boards_cli::context;
use azure_boards_cli::error::CliError;
use azure_boards_cli::output::{self, CommandOutput, OutputFormat};
use clap::Parser;

#[tokio::main]
async fn main() {
    // clap exits 2 on usage errors and 0 for --help/--version by itself.
    let cli = Cli::parse();
    init_logging(&cli.global);

    let stdout_tty = output::stdout_is_tty();
    let inputs = context::gather_inputs(
        cli.global.org.clone(),
        cli.global.project.clone(),
        cli.global.team.clone(),
        cli.global.detect,
    );

    let format_flag = if cli.global.json {
        Some(OutputFormat::Json)
    } else {
        cli.global.output
    };

    let (mut ctx, explanation) = match context::resolve(&inputs) {
        Ok(v) => v,
        Err(e) => exit_with(e, format_flag.unwrap_or(OutputFormat::Json)),
    };
    ctx.assume_yes = cli.global.yes;
    let format = output::select_format(format_flag, ctx.config_output, stdout_tty);

    let result = dispatch(&cli, &ctx, &explanation).await;
    match result {
        Ok(out) => {
            if let Err(e) = output::render(&out, format, cli.global.query.as_deref(), stdout_tty) {
                exit_with(e, format);
            }
        }
        Err(e) => exit_with(e, format),
    }
}

fn exit_with(err: CliError, format: OutputFormat) -> ! {
    output::emit_error(&err, format);
    std::process::exit(err.exit_code() as i32);
}

fn init_logging(global: &azure_boards_cli::cli::GlobalArgs) {
    let level = if global.debug {
        "debug"
    } else if global.verbose {
        "info"
    } else if global.only_show_errors {
        "error"
    } else {
        "warn"
    };
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(level));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .without_time()
        .init();
}

async fn dispatch(
    cli: &Cli,
    ctx: &context::Ctx,
    explanation: &context::Explanation,
) -> Result<CommandOutput, CliError> {
    match &cli.command {
        Commands::Completion { shell } => {
            use clap::CommandFactory;
            let bin = std::env::args()
                .next()
                .map(|p| {
                    std::path::Path::new(&p)
                        .file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "azure-boards".into())
                })
                .unwrap_or_else(|| "azure-boards".into());
            clap_complete::generate(*shell, &mut Cli::command(), bin, &mut std::io::stdout());
            Ok(CommandOutput::None)
        }
        Commands::Context { cmd } => commands::context_cmd::run(cmd, ctx, explanation),
        Commands::Auth { cmd } => commands::auth_cmd::run(cmd, ctx).await,
        Commands::Configure(args) => commands::configure::run(args),
        Commands::WorkItem { cmd } => {
            let clients = commands::make_clients(ctx).await?;
            commands::work_item::run(cmd, ctx, &clients).await
        }
        Commands::Wiql { cmd } => {
            let clients = commands::make_clients(ctx).await?;
            commands::wiql_cmd::run(cmd, ctx, &clients).await
        }
        Commands::Tag { cmd } => {
            let clients = commands::make_clients(ctx).await?;
            commands::tag_cmd::run(cmd, ctx, &clients).await
        }
        Commands::Comment { cmd } => {
            let clients = commands::make_clients(ctx).await?;
            commands::comment::run(cmd, ctx, &clients).await
        }
        Commands::Relation { cmd } => {
            let clients = commands::make_clients(ctx).await?;
            commands::relation::run(cmd, ctx, &clients).await
        }
        Commands::Attachment { cmd } => {
            let clients = commands::make_clients(ctx).await?;
            commands::attachment::run(cmd, ctx, &clients).await
        }
        Commands::Image { cmd } => {
            let clients = commands::make_clients(ctx).await?;
            commands::image::run(cmd, ctx, &clients).await
        }
        Commands::Area { cmd } => {
            let clients = commands::make_clients(ctx).await?;
            commands::area::run(cmd, ctx, &clients).await
        }
        Commands::Iteration { cmd } => {
            let clients = commands::make_clients(ctx).await?;
            commands::iteration::run(cmd, ctx, &clients).await
        }
        Commands::Sprint { cmd } => {
            let clients = commands::make_clients(ctx).await?;
            commands::sprint::run(cmd, ctx, &clients).await
        }
        Commands::Board { cmd } => {
            let clients = commands::make_clients(ctx).await?;
            commands::board::run(cmd, ctx, &clients).await
        }
        Commands::Backlog { cmd } => {
            let clients = commands::make_clients(ctx).await?;
            commands::backlog::run(cmd, ctx, &clients).await
        }
        Commands::Query { cmd } => {
            let clients = commands::make_clients(ctx).await?;
            commands::query::run(cmd, ctx, &clients).await
        }
        Commands::Metrics { cmd } => {
            let clients = commands::make_clients(ctx).await?;
            commands::metrics::run(cmd, ctx, &clients).await
        }
        Commands::Github { cmd } => {
            let clients = commands::make_clients(ctx).await?;
            commands::github::run(cmd, ctx, &clients).await
        }
    }
}
