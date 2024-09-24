use crate::admin::AdminRpc;
use crate::cli::{
	cmd_apply_layout, cmd_assign_role, fetch_layout, fetch_status, ApplyLayoutOpt, AssignRoleOpt,
	BucketOperation, BucketOpt, KeyImportOpt, KeyInfoOpt, KeyOperation, PermBucketOpt,
};
use bytesize::ByteSize;
use garage_model::helper::error::Error as HelperError;
use garage_net::endpoint::Endpoint;
use garage_net::message::PRIO_NORMAL;
use garage_net::NodeID;
use garage_rpc::layout::NodeRoleV;
use garage_rpc::system::SystemRpc;
use garage_util::config::{AutoBucket, AutoKey, AutoNode, AutoPermission};
use garage_util::data::Uuid;
use garage_util::error::Error;

pub async fn key_exists(
	rpc_cli: &Endpoint<AdminRpc, ()>,
	rpc_host: NodeID,
	key_pattern: String,
) -> Result<bool, Error> {
	match rpc_cli
		.call(
			&rpc_host,
			AdminRpc::KeyOperation(KeyOperation::Info(KeyInfoOpt {
				key_pattern,
				show_secret: false,
			})),
			PRIO_NORMAL,
		)
		.await?
	{
		Ok(_) => Ok(true),
		Err(HelperError::BadRequest(_)) => Ok(false),
		resp => Err(Error::unexpected_rpc_message(resp)),
	}
}

pub async fn bucket_exists(
	rpc_cli: &Endpoint<AdminRpc, ()>,
	rpc_host: NodeID,
	name: String,
) -> Result<bool, Error> {
	match rpc_cli
		.call(
			&rpc_host,
			AdminRpc::BucketOperation(BucketOperation::Info(BucketOpt { name })),
			PRIO_NORMAL,
		)
		.await?
	{
		Ok(_) => Ok(true),
		Err(HelperError::BadRequest(_)) => Ok(false),
		resp => Err(Error::unexpected_rpc_message(resp)),
	}
}

pub async fn key_create(
	rpc_cli: &Endpoint<AdminRpc, ()>,
	rpc_host: NodeID,
	params: &AutoKey,
) -> Result<(), Error> {
	match rpc_cli
		.call(
			&rpc_host,
			AdminRpc::KeyOperation(KeyOperation::Import(KeyImportOpt {
				name: params.name.clone(),
				secret_key: params.secret.clone(),
				key_id: params.id.clone(),
				yes: true,
			})),
			PRIO_NORMAL,
		)
		.await?
	{
		Ok(_) => Ok(()),
		Err(HelperError::BadRequest(msg)) => Err(Error::Message(msg)),
		resp => Err(Error::unexpected_rpc_message(resp)),
	}
}

pub async fn bucket_create(
	rpc_cli: &Endpoint<AdminRpc, ()>,
	rpc_host: NodeID,
	params: &AutoBucket,
) -> Result<(), Error> {
	match rpc_cli
		.call(
			&rpc_host,
			AdminRpc::BucketOperation(BucketOperation::Create(BucketOpt {
				name: params.name.clone(),
			})),
			PRIO_NORMAL,
		)
		.await?
	{
		Ok(_) => Ok(()),
		Err(HelperError::BadRequest(msg)) => Err(Error::Message(msg)),
		resp => Err(Error::unexpected_rpc_message(resp)),
	}
}

pub async fn grant_permission(
	rpc_cli: &Endpoint<AdminRpc, ()>,
	rpc_host: NodeID,
	bucket_name: String,
	perm: &AutoPermission,
) -> Result<(), Error> {
	match rpc_cli
		.call(
			&rpc_host,
			AdminRpc::BucketOperation(BucketOperation::Allow(PermBucketOpt {
				key_pattern: perm.key.clone(),
				read: perm.read,
				write: perm.write,
				owner: perm.owner,
				bucket: bucket_name,
			})),
			PRIO_NORMAL,
		)
		.await?
	{
		Ok(_) => Ok(()),
		Err(HelperError::BadRequest(msg)) => Err(Error::Message(msg)),
		resp => Err(Error::unexpected_rpc_message(resp)),
	}
}

pub async fn get_unassigned_nodes(
	rpc_cli: &Endpoint<SystemRpc, ()>,
	rpc_host: NodeID,
) -> Result<Option<Vec<Uuid>>, Error> {
	let status = fetch_status(rpc_cli, rpc_host).await?;
	let layout = fetch_layout(rpc_cli, rpc_host).await?;
	let mut nodes: Vec<Uuid> = Vec::new();

	for adv in status.iter().filter(|adv| adv.is_up) {
		if layout.current().roles.get(&adv.id).is_none() {
			let prev_role = layout
				.versions
				.iter()
				.rev()
				.find_map(|x| match x.roles.get(&adv.id) {
					Some(NodeRoleV(Some(cfg))) => Some(cfg),
					_ => None,
				});
			if prev_role.is_none() {
				if let Some(NodeRoleV(Some(_))) = layout.staging.get().roles.get(&adv.id) {
					// Node role assignment is pending, can return immediately.
					return Ok(None);
				} else {
					nodes.push(adv.id.clone());
				}
			}
		} else {
			// Node role is assigned, can return immediately.
			return Ok(None);
		}
	}

	// Encountered no node with an assignment (pending or applied).
	// Therefore, all nodes are unassigned.
	Ok(Some(nodes))
}

pub async fn assign_node_layout(
	rpc_cli: &Endpoint<SystemRpc, ()>,
	rpc_host: NodeID,
	unassigned_nodes: &Vec<Uuid>,
	auto_nodes: &Vec<AutoNode>,
) -> Result<(), Error> {
	if unassigned_nodes.len() != auto_nodes.len() {
		return Err(Error::Message(
			"Cannot apply auto layout: configured nodes do not match actual nodes".to_string(),
		));
	}

	for (i, node_id) in unassigned_nodes.iter().enumerate() {
		if let Some(auto) = auto_nodes.get(i) {
			let capacity = auto.capacity.parse::<ByteSize>()?;
			cmd_assign_role(
				rpc_cli,
				rpc_host,
				AssignRoleOpt {
					node_ids: vec![format!("{id:?}", id = node_id)],
					zone: Some(auto.zone.clone()),
					capacity: Some(capacity),
					gateway: false,
					tags: vec![],
					replace: vec![],
				},
			)
			.await?;
		}
	}

	cmd_apply_layout(rpc_cli, rpc_host, ApplyLayoutOpt { version: Some(1) }).await?;

	Ok(())
}
