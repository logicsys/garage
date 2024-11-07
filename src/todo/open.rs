use std::path::PathBuf;

use crate::{Todo, Result, Error};

/// List of supported todo engine types
///
/// The `enum` holds list of *all* todo engines that are are be supported by crate, no matter
/// if relevant feature is enabled or not. It allows us to distinguish between invalid engine
/// and valid engine, whose support is not enabled via feature flag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Engine {
	Yaque,
}

impl Engine {
	/// Return variant name as static `&str`
	pub fn as_str(&self) -> &'static str {
		match self {
			Self::Yaque => "yaque",
		}
	}
}

impl std::fmt::Display for Engine {
	fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
		self.as_str().fmt(fmt)
	}
}

impl std::str::FromStr for Engine {
	type Err = Error;

	fn from_str(text: &str) -> Result<Engine> {
		match text {
		    #[cfg(feature = "yaque")]
			"yaque" => Ok(Self::Yaque),
			kind => Err(Error(
				format!(
					"Invalid todo engine: {} (options are: yaque)",
					kind
				)
				.into(),
			)),
		}
	}
}

pub fn open_todo(path: &PathBuf, engine: Engine) -> Result<Todo> {
	match engine {
		// ---- Yaque ----
		#[cfg(feature = "yaque")]
		Engine::Yaque => {
			info!("Opening Yaque todo at: {}", path.display());
			let engine = crate::yaque_adapter::Yaque::init(path);
			Ok(engine)
		}

		// Pattern is unreachable when all supported todo engines are compiled into binary. The allow
		// attribute is added so that we won't have to change this match in case stop building
		// support for one or more engines by default.
		#[allow(unreachable_patterns)]
		engine => Err(Error(
			format!("Todo engine support not available in this build: {}", engine).into(),
		)),
	}
}
