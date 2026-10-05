//! Capability negotiation for custom provider plugins. Native hosts compile their
//! own requests; an older custom plugin receives its ordinary request unchanged.

use std::time::{Duration, Instant};

use serde_json::{Map, Value, json};

use crate::{
    journal::diagnostic_value_summary,
    permissions::{
        InvocationPermissions, PermissionApplicationStatus, PermissionCapabilities,
        PermissionCapability, PermissionField, PermissionReport, PermissionSupport,
        PermissionWarning,
    },
    process::{CancellationToken, InvocationKind, ProcessError, ProcessSpec, ProcessSupervisor},
    protocol::{JsonField, PluginEvent, PluginFrame, PluginRequest, TerminalResult},
};

use super::CliError;

pub(super) fn negotiate(
    spec: &mut ProcessSpec,
    request: &PluginRequest,
    cancellation: &CancellationToken,
) -> Result<(PluginRequest, Option<PermissionReport>), CliError> {
    let mut forwarded = request.clone();
    let Some(value) = forwarded.params.remove("permissions") else {
        return Ok((forwarded, None));
    };
    let requested: InvocationPermissions = serde_json::from_value(value)?;
    let mut report = PermissionReport::new(requested.clone());
    if requested == InvocationPermissions::default() {
        return Ok((forwarded, Some(report)));
    }
    // A describe is an offline capability query, never a retry of invoke. Its
    // bounded cost is charged to the same overall supervisor deadline.
    let started = Instant::now();
    let mut probe = spec.clone();
    probe.limits.deadline = Some(
        spec.limits
            .deadline
            .unwrap_or(Duration::from_secs(5))
            .min(Duration::from_secs(5)),
    );
    probe.limits.max_frame_bytes = probe.limits.max_frame_bytes.min(64 * 1024);
    probe.limits.max_stdout_bytes = probe.limits.max_stdout_bytes.min(128 * 1024);
    probe.limits.max_frames = 16;
    let mut params = Map::new();
    if let Some(workspace) = request.params.get("workspace") {
        params.insert("workspace".to_owned(), workspace.clone());
    }
    if let Some(options) = request.params.get("options") {
        params.insert("options".to_owned(), options.clone());
    }
    params.insert("permissions".to_owned(), serde_json::to_value(&requested)?);
    let description = PluginRequest {
        api: request.api.clone(),
        id: request.id.clone(),
        method: "describe".to_owned(),
        params,
    };
    let result = if cancellation.is_cancelled() || spec.limits.deadline == Some(Duration::ZERO) {
        Err(ProcessError::InvalidSpec(
            "permission negotiation cancelled before dispatch".to_owned(),
        ))
    } else {
        ProcessSupervisor.invoke(&probe, &description, cancellation, |_| {})
    };
    if let Some(deadline) = &mut spec.limits.deadline {
        *deadline = deadline.saturating_sub(started.elapsed());
    }
    let capabilities = result
        .ok()
        .filter(|outcome| outcome.kind == InvocationKind::Succeeded)
        .and_then(|outcome| match outcome.terminal {
            Some(TerminalResult::Success { value }) => {
                value.get("permissions_capabilities").cloned()
            }
            _ => None,
        })
        .and_then(|value| serde_json::from_value::<PermissionCapabilities>(value).ok());
    let missing = "The provider did not return a valid permissions_capabilities declaration; this field uses the provider's normal mode.";
    for (field, present) in [
        (
            PermissionField::AllowedTools,
            requested.allowed_tools.is_some(),
        ),
        (
            PermissionField::AllowedSkills,
            requested.allowed_skills.is_some(),
        ),
        (PermissionField::Sandbox, requested.sandbox.is_some()),
    ] {
        if !present {
            continue;
        }
        let Some(capabilities) = capabilities.as_ref() else {
            report.unsupported(field, missing);
            continue;
        };
        let capability = match field {
            PermissionField::AllowedTools => &capabilities.allowed_tools,
            PermissionField::AllowedSkills => &capabilities.allowed_skills,
            PermissionField::Sandbox => &capabilities.sandbox,
        };
        apply_capability(&mut report, field, capability, &requested);
    }
    if report.effective != InvocationPermissions::default() {
        forwarded.params.insert(
            "permissions".to_owned(),
            serde_json::to_value(&report.effective)?,
        );
    }
    Ok((forwarded, Some(report)))
}

fn apply_capability(
    report: &mut PermissionReport,
    field: PermissionField,
    capability: &PermissionCapability,
    requested: &InvocationPermissions,
) {
    if capability.support == PermissionSupport::Unsupported {
        report.unsupported(
            field,
            capability.reason.as_deref().unwrap_or(
                "The provider does not support this permission; normal mode is retained.",
            ),
        );
        return;
    }
    let mut effective = requested.clone();
    let mut partial_reason = None;
    if field == PermissionField::Sandbox {
        let sandbox = effective.sandbox.as_mut().expect("requested sandbox");
        if capability
            .modes
            .as_ref()
            .is_some_and(|modes| !modes.contains(&sandbox.mode))
        {
            report.unsupported(field, "The provider does not support the requested sandbox mode; normal mode is retained.");
            return;
        }
        if sandbox.workspace.is_some() && capability.workspace != Some(true) {
            sandbox.workspace = None;
            partial_reason = Some(
                "The provider supports the sandbox mode but not a custom sandbox workspace; its normal workspace is retained.",
            );
        }
    }
    if capability.support == PermissionSupport::Partial || partial_reason.is_some() {
        report.partial(
            field,
            &effective,
            capability.scope.as_deref().unwrap_or("plugin_declared"),
            partial_reason
                .or(capability.reason.as_deref())
                .unwrap_or("The provider declares only partial support for this permission."),
        );
    } else {
        report.record(
            field,
            PermissionApplicationStatus::Applied,
            &effective,
            capability.scope.as_deref(),
            capability.reason.as_deref(),
        );
    }
}

pub(super) fn events(request: &PluginRequest, report: &PermissionReport) -> Vec<PluginFrame> {
    let mut result = Vec::new();
    for warning in &report.warnings {
        result.push(PluginFrame::Event {
            id: request.id.clone(),
            event: PluginEvent {
                kind: "provider.permission.warning".to_owned(),
                payload: serde_json::to_value(warning)
                    .expect("serializable warning")
                    .as_object()
                    .expect("warning object")
                    .clone(),
            },
        });
    }
    result.push(PluginFrame::Event {
        id: request.id.clone(),
        event: PluginEvent {
            kind: "provider.permissions".to_owned(),
            payload: Map::from_iter([("permission_report".to_owned(), json!(report))]),
        },
    });
    result
}

pub(super) fn annotate_frame(frame: &mut PluginFrame, report: &PermissionReport) {
    match frame {
        PluginFrame::Result {
            value: JsonField::Present(value),
            ok: true,
            ..
        } => annotate_value(value, report),
        PluginFrame::Result {
            error: JsonField::Present(error),
            ok: false,
            ..
        } => {
            let mut details = Map::new();
            if let Some(original) = error.details.take() {
                details.insert("provider_details".to_owned(), original);
            }
            details.insert("permission_report".to_owned(), json!(report));
            error.details = Some(Value::Object(details));
        }
        _ => {}
    }
}

pub(super) fn annotate_terminal(terminal: &mut TerminalResult, report: &PermissionReport) {
    match terminal {
        TerminalResult::Success { value } => annotate_value(value, report),
        TerminalResult::Failure { error } => {
            let mut details = Map::new();
            if let Some(original) = error.details.take() {
                details.insert("provider_details".to_owned(), original);
            }
            details.insert("permission_report".to_owned(), json!(report));
            error.details = Some(Value::Object(details));
        }
    }
}

fn annotate_value(value: &mut Value, report: &PermissionReport) {
    // Scalar provider values retain their ABI; the report is still an event and
    // a journal record. Object providers reserve this runtime metadata field.
    if let Some(object) = value.as_object_mut() {
        if let Some(receipt) = object.remove("permission_report") {
            object.insert("provider_permission_report".to_owned(), receipt);
        }
        object.insert("permission_report".to_owned(), json!(report));
        object.insert(
            "permission_report_basis".to_owned(),
            json!("capability_declaration"),
        );
    }
}

pub(super) fn diagnostic_event(kind: &str, payload: &Map<String, Value>) -> Option<Value> {
    match kind {
        "provider.permission.warning" => {
            let warning: PermissionWarning =
                serde_json::from_value(Value::Object(payload.clone())).ok()?;
            Some(
                json!({"event_type":kind,"code":warning.code,"field":warning.field,
                "message_summary":diagnostic_value_summary(&json!(warning.message))}),
            )
        }
        "provider.permissions" => {
            let report: PermissionReport =
                serde_json::from_value(payload.get("permission_report")?.clone()).ok()?;
            Some(json!({"event_type":kind,
                "requested_summary":diagnostic_value_summary(&json!(report.requested)),
                "effective_summary":diagnostic_value_summary(&json!(report.effective)),
                "applications": report.applications.iter().map(|application| json!({
                    "field":application.field,"status":application.status,
                    "scope_summary":diagnostic_value_summary(&json!(application.scope)),
                    "reason_summary":diagnostic_value_summary(&json!(application.reason))
                })).collect::<Vec<_>>(),
                "warnings":report.warnings.iter().map(|warning| json!({"code":warning.code,"field":warning.field})).collect::<Vec<_>>()
            }))
        }
        _ => None,
    }
}
