use crate::cli::ContextCmd;
use crate::context::{Ctx, Explanation};
use crate::error::CliError;
use crate::output::CommandOutput;
use serde_json::json;

pub fn run(
    cmd: &ContextCmd,
    ctx: &Ctx,
    explanation: &Explanation,
) -> Result<CommandOutput, CliError> {
    match cmd {
        ContextCmd::Show => Ok(CommandOutput::Item {
            value: json!({
                "organization": ctx.org.as_ref().map(|r| &r.value),
                "organizationSource": ctx.org.as_ref().map(|r| r.source.describe()),
                "project": ctx.project.as_ref().map(|r| &r.value),
                "projectSource": ctx.project.as_ref().map(|r| r.source.describe()),
                "team": ctx.team.as_ref().map(|r| &r.value),
                "teamSource": ctx.team.as_ref().map(|r| r.source.describe()),
                "detect": ctx.detect_enabled.value,
                "githubRepo": ctx
                    .github_repo
                    .as_ref()
                    .map(|g| format!("{}/{}", g.owner, g.repo)),
            }),
            columns: vec![],
        }),
        ContextCmd::Detect { explain } => {
            if *explain {
                Ok(CommandOutput::Item {
                    value: explanation.to_json(),
                    columns: vec![],
                })
            } else {
                Ok(CommandOutput::Item {
                    value: json!({
                        "organization": ctx.org.as_ref().map(|r| &r.value),
                        "project": ctx.project.as_ref().map(|r| &r.value),
                        "githubRepo": ctx
                            .github_repo
                            .as_ref()
                            .map(|g| format!("{}/{}", g.owner, g.repo)),
                    }),
                    columns: vec![],
                })
            }
        }
    }
}
