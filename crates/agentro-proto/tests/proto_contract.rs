use std::{collections::BTreeSet, error::Error};

use agentro_proto::{FILE_DESCRIPTOR_SET, agentro::common::v1::PageRequest};
use prost::Message;

#[test]
fn generated_message_round_trips_optional_presence() -> Result<(), Box<dyn Error>> {
    let request = PageRequest {
        page_size: Some(25),
        page_token: vec![1, 2, 3],
    };
    let encoded = request.encode_to_vec();
    let decoded = PageRequest::decode(encoded.as_slice())?;

    assert_eq!(decoded, request);
    Ok(())
}

#[test]
fn descriptor_contains_all_alpha_bounded_contexts() -> Result<(), Box<dyn Error>> {
    let descriptor = prost_types::FileDescriptorSet::decode(FILE_DESCRIPTOR_SET)?;
    let packages: BTreeSet<&str> = descriptor
        .file
        .iter()
        .filter_map(|file| file.package.as_deref())
        .collect();

    assert!(packages.contains("agentro.common.v1"));
    assert!(packages.contains("agentro.execution.v1"));
    assert!(packages.contains("agentro.schedule.v1"));
    assert!(packages.contains("agentro.system.v1"));
    assert!(packages.contains("agentro.workflow.v1"));
    assert!(packages.contains("agentro.workspace.v1"));
    let services: BTreeSet<&str> = descriptor
        .file
        .iter()
        .flat_map(|file| file.service.iter())
        .filter_map(|service| service.name.as_deref())
        .collect();
    assert_eq!(
        services,
        BTreeSet::from([
            "RunService",
            "ScheduleService",
            "SystemService",
            "WorkflowService",
            "WorkspaceService",
        ])
    );
    Ok(())
}
