use hyper::{Request, Response, StatusCode};

use garage_table::EmptyKey;
use garage_util::time::*;

use garage_model::bucket_table::*;
use garage_model::s3::object_table::*;

use garage_api_common::helpers::*;

use crate::api_server::{ReqBody, ResBody};
use crate::error::*;

// ---- GetObjectLockConfiguration ----

pub async fn handle_get_object_lock_configuration(
	ctx: ReqCtx,
) -> Result<Response<ResBody>, Error> {
	let config = ctx.bucket_params.object_lock_config.get();

	match config {
		Some(olc) if olc.enabled => {
			let mut xml = r#"<?xml version="1.0" encoding="UTF-8"?><ObjectLockConfiguration xmlns="http://s3.amazonaws.com/doc/2006-03-01/"><ObjectLockEnabled>Enabled</ObjectLockEnabled>"#.to_string();

			if let Some(ref ret) = olc.default_retention {
				xml.push_str("<Rule><DefaultRetention>");
				match ret.mode {
					ObjectLockRetentionMode::Governance => {
						xml.push_str("<Mode>GOVERNANCE</Mode>");
					}
					ObjectLockRetentionMode::Compliance => {
						xml.push_str("<Mode>COMPLIANCE</Mode>");
					}
				}
				if let Some(days) = ret.days {
					xml.push_str(&format!("<Days>{}</Days>", days));
				}
				if let Some(years) = ret.years {
					xml.push_str(&format!("<Years>{}</Years>", years));
				}
				xml.push_str("</DefaultRetention></Rule>");
			}

			xml.push_str("</ObjectLockConfiguration>");

			Ok(Response::builder()
				.header("Content-Type", "application/xml")
				.body(string_body(xml))?)
		}
		_ => Err(Error::ObjectLockConfigurationNotFound),
	}
}

// ---- PutObjectLockConfiguration ----

pub async fn handle_put_object_lock_configuration(
	ctx: ReqCtx,
	req: Request<ReqBody>,
) -> Result<Response<ResBody>, Error> {
	// Bucket must already have object lock enabled
	let current_config = ctx.bucket_params.object_lock_config.get();
	match current_config {
		Some(olc) if olc.enabled => {}
		_ => {
			return Err(Error::ObjectLockConfigurationNotFound);
		}
	}

	let body = req.into_body().collect().await?;
	let xml = roxmltree::Document::parse(std::str::from_utf8(&body)?)?;

	let root = xml.root().first_child().ok_or_bad_request("Missing root element")?;
	if !root.has_tag_name("ObjectLockConfiguration") {
		return Err(Error::bad_request("Expected ObjectLockConfiguration element"));
	}

	let default_retention = parse_default_retention(&root)?;

	let new_config = ObjectLockConfiguration {
		enabled: true,
		default_retention,
	};

	let mut bucket = ctx
		.garage
		.bucket_table
		.get(&EmptyKey, &ctx.bucket_id)
		.await?
		.ok_or_internal_error("Bucket not found")?;

	if let Some(params) = bucket.params_mut() {
		params.object_lock_config.update(Some(new_config));
	}
	ctx.garage.bucket_table.insert(&bucket).await?;

	Ok(Response::builder()
		.status(StatusCode::OK)
		.body(empty_body())?)
}

fn parse_default_retention(
	root: &roxmltree::Node,
) -> Result<Option<ObjectLockRetention>, Error> {
	let rule = match root.children().find(|c| c.has_tag_name("Rule")) {
		Some(r) => r,
		None => return Ok(None),
	};

	let dr = rule
		.children()
		.find(|c| c.has_tag_name("DefaultRetention"))
		.ok_or_bad_request("Missing DefaultRetention in Rule")?;

	let mode_str = dr
		.children()
		.find(|c| c.has_tag_name("Mode"))
		.and_then(|n| n.text())
		.ok_or_bad_request("Missing Mode in DefaultRetention")?;

	let mode = match mode_str {
		"GOVERNANCE" => ObjectLockRetentionMode::Governance,
		"COMPLIANCE" => ObjectLockRetentionMode::Compliance,
		_ => return Err(Error::bad_request(format!("Invalid retention mode: {}", mode_str))),
	};

	let days = dr
		.children()
		.find(|c| c.has_tag_name("Days"))
		.and_then(|n| n.text())
		.map(|t| t.parse::<u64>())
		.transpose()
		.map_err(|_| Error::bad_request("Invalid Days value"))?;

	let years = dr
		.children()
		.find(|c| c.has_tag_name("Years"))
		.and_then(|n| n.text())
		.map(|t| t.parse::<u64>())
		.transpose()
		.map_err(|_| Error::bad_request("Invalid Years value"))?;

	if days.is_none() && years.is_none() {
		return Err(Error::bad_request(
			"DefaultRetention must specify either Days or Years",
		));
	}
	if days.is_some() && years.is_some() {
		return Err(Error::bad_request(
			"DefaultRetention cannot specify both Days and Years",
		));
	}

	Ok(Some(ObjectLockRetention { mode, days, years }))
}

// ---- GetObjectRetention ----

pub async fn handle_get_object_retention(
	ctx: ReqCtx,
	key: &str,
) -> Result<Response<ResBody>, Error> {
	let object = ctx
		.garage
		.object_table
		.get(&ctx.bucket_id, &key.to_string())
		.await?
		.ok_or(Error::NoSuchKey)?;

	match object.retention.get() {
		Some(retention) => {
			let mode_str = match retention.mode {
				ObjectRetentionMode::Governance => "GOVERNANCE",
				ObjectRetentionMode::Compliance => "COMPLIANCE",
			};
			let date = msec_to_rfc3339(retention.retain_until);

			let xml = format!(
				r#"<?xml version="1.0" encoding="UTF-8"?><Retention xmlns="http://s3.amazonaws.com/doc/2006-03-01/"><Mode>{}</Mode><RetainUntilDate>{}</RetainUntilDate></Retention>"#,
				mode_str, date
			);

			Ok(Response::builder()
				.header("Content-Type", "application/xml")
				.body(string_body(xml))?)
		}
		None => Err(Error::bad_request(
			"The specified object does not have a retention configuration",
		)),
	}
}

// ---- PutObjectRetention ----

pub async fn handle_put_object_retention(
	ctx: ReqCtx,
	req: Request<ReqBody>,
	key: &str,
) -> Result<Response<ResBody>, Error> {
	let bypass_governance = req
		.headers()
		.get("x-amz-bypass-governance-retention")
		.map(|v| v.to_str().unwrap_or("") == "true")
		.unwrap_or(false);

	let body = req.into_body().collect().await?;
	let xml = roxmltree::Document::parse(std::str::from_utf8(&body)?)?;

	let root = xml.root().first_child().ok_or_bad_request("Missing root")?;
	if !root.has_tag_name("Retention") {
		return Err(Error::bad_request("Expected Retention element"));
	}

	let mode_str = root
		.children()
		.find(|c| c.has_tag_name("Mode"))
		.and_then(|n| n.text());

	let retain_until_str = root
		.children()
		.find(|c| c.has_tag_name("RetainUntilDate"))
		.and_then(|n| n.text());

	let new_retention = match (mode_str, retain_until_str) {
		(Some(mode), Some(date)) => {
			let mode = match mode {
				"GOVERNANCE" => ObjectRetentionMode::Governance,
				"COMPLIANCE" => ObjectRetentionMode::Compliance,
				_ => {
					return Err(Error::InvalidRetention(format!(
						"Invalid mode: {}",
						mode
					)));
				}
			};
			let retain_until = chrono::DateTime::parse_from_rfc3339(date)
				.map_err(|e| Error::InvalidRetention(format!("Invalid date: {}", e)))?
				.timestamp_millis() as u64;
			Some(ObjectRetention {
				mode,
				retain_until,
			})
		}
		(None, None) => None, // Remove retention
		_ => {
			return Err(Error::InvalidRetention(
				"Both Mode and RetainUntilDate must be specified".to_string(),
			));
		}
	};

	let mut object = ctx
		.garage
		.object_table
		.get(&ctx.bucket_id, &key.to_string())
		.await?
		.ok_or(Error::NoSuchKey)?;

	// Check current retention rules
	if let Some(current) = object.retention.get() {
		let now = now_msec();
		if now < current.retain_until {
			match current.mode {
				ObjectRetentionMode::Compliance => {
					// Cannot shorten, remove, or change mode of compliance retention
					if let Some(ref new) = new_retention {
						if new.mode != ObjectRetentionMode::Compliance {
							return Err(Error::ObjectLocked(
								"Cannot change compliance mode retention to another mode".to_string(),
							));
						}
						if new.retain_until < current.retain_until {
							return Err(Error::ObjectLocked(
								"Cannot shorten compliance mode retention".to_string(),
							));
						}
					} else {
						return Err(Error::ObjectLocked(
							"Cannot remove compliance mode retention".to_string(),
						));
					}
				}
				ObjectRetentionMode::Governance => {
					if !bypass_governance {
						return Err(Error::ObjectLocked(
							"Object is under governance retention. Use x-amz-bypass-governance-retention header to override.".to_string(),
						));
					}
				}
			}
		}
	}

	object.retention.update(new_retention);
	ctx.garage.object_table.insert(&object).await?;

	Ok(Response::builder()
		.status(StatusCode::OK)
		.body(empty_body())?)
}

// ---- GetObjectLegalHold ----

pub async fn handle_get_object_legal_hold(
	ctx: ReqCtx,
	key: &str,
) -> Result<Response<ResBody>, Error> {
	let object = ctx
		.garage
		.object_table
		.get(&ctx.bucket_id, &key.to_string())
		.await?
		.ok_or(Error::NoSuchKey)?;

	let status = match object.legal_hold.get() {
		Some(true) => "ON",
		_ => "OFF",
	};

	let xml = format!(
		r#"<?xml version="1.0" encoding="UTF-8"?><LegalHold xmlns="http://s3.amazonaws.com/doc/2006-03-01/"><Status>{}</Status></LegalHold>"#,
		status
	);

	Ok(Response::builder()
		.header("Content-Type", "application/xml")
		.body(string_body(xml))?)
}

// ---- PutObjectLegalHold ----

pub async fn handle_put_object_legal_hold(
	ctx: ReqCtx,
	req: Request<ReqBody>,
	key: &str,
) -> Result<Response<ResBody>, Error> {
	let body = req.into_body().collect().await?;
	let xml = roxmltree::Document::parse(std::str::from_utf8(&body)?)?;

	let root = xml.root().first_child().ok_or_bad_request("Missing root")?;
	if !root.has_tag_name("LegalHold") {
		return Err(Error::bad_request("Expected LegalHold element"));
	}

	let status = root
		.children()
		.find(|c| c.has_tag_name("Status"))
		.and_then(|n| n.text())
		.ok_or_bad_request("Missing Status element")?;

	let hold = match status {
		"ON" => Some(true),
		"OFF" => Some(false),
		_ => return Err(Error::bad_request(format!("Invalid legal hold status: {}", status))),
	};

	let mut object = ctx
		.garage
		.object_table
		.get(&ctx.bucket_id, &key.to_string())
		.await?
		.ok_or(Error::NoSuchKey)?;

	object.legal_hold.update(hold);
	ctx.garage.object_table.insert(&object).await?;

	Ok(Response::builder()
		.status(StatusCode::OK)
		.body(empty_body())?)
}
