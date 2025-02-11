use anyhow::bail;
use anyhow::Result;
use dprint_core::configuration::ConfigKeyMap;
use std::borrow::Cow;
use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use crate::environment::Environment;
use crate::incremental::IncrementalFile;
use crate::paths::PluginNames;
use crate::plugins::do_batch_format;
use crate::plugins::PluginAndPoolMutRef;
use crate::plugins::PluginPools;
use crate::plugins::TakePluginResult;
use crate::utils::ErrorCountLogger;
use crate::utils::FileText;

pub fn format_with_plugin_pools<'a, TEnvironment: Environment>(
  file_name: &Path,
  file_text: &'a str,
  environment: &TEnvironment,
  plugin_pools: &Arc<PluginPools<TEnvironment>>,
) -> Result<Cow<'a, str>> {
  let plugin_names = plugin_pools.get_plugin_names_from_file_name(file_name);
  let mut file_text = Cow::Borrowed(file_text);
  for plugin_name in plugin_names {
    let plugin_pool = plugin_pools.get_pool(&plugin_name).unwrap();
    let error_logger = ErrorCountLogger::from_environment(environment);
    match plugin_pool.take_or_create_checking_config_diagnostics(&error_logger)? {
      TakePluginResult::Success(mut initialized_plugin) => {
        let result = initialized_plugin.format_text(file_name, &file_text, &ConfigKeyMap::new());
        plugin_pool.release(initialized_plugin);
        file_text = Cow::Owned(result?); // release plugin above, then propagate this error
      }
      TakePluginResult::HadDiagnostics => {
        bail!("Had {} configuration errors.", error_logger.get_error_count());
      }
    }
  }
  Ok(file_text)
}

pub fn run_parallelized<F, TEnvironment: Environment>(
  file_paths_by_plugins: HashMap<PluginNames, Vec<PathBuf>>,
  environment: &TEnvironment,
  plugin_pools: Arc<PluginPools<TEnvironment>>,
  incremental_file: Option<Arc<IncrementalFile<TEnvironment>>>,
  f: F,
) -> Result<()>
where
  F: Fn(&Path, &str, String, bool, Instant, &TEnvironment) -> Result<()> + Send + 'static + Clone,
{
  let error_logger = ErrorCountLogger::from_environment(environment);

  do_batch_format(environment, &error_logger, &plugin_pools, file_paths_by_plugins, {
    let environment = environment.clone();
    let error_logger = error_logger.clone();
    move |plugins, file_path| {
      let result = run_for_file_path(&environment, &incremental_file, plugins, file_path, f.clone());
      if let Err(err) = result {
        error_logger.log_error(&format!("Error formatting {}. Message: {}", file_path.display(), err));
      }
    }
  })?;

  let error_count = error_logger.get_error_count();
  return if error_count == 0 {
    Ok(())
  } else {
    bail!("Had {0} error(s) formatting.", error_count)
  };

  #[inline]
  fn run_for_file_path<F, TEnvironment: Environment>(
    environment: &TEnvironment,
    incremental_file: &Option<Arc<IncrementalFile<TEnvironment>>>,
    mut plugins: Vec<PluginAndPoolMutRef<TEnvironment>>,
    file_path: &Path,
    f: F,
  ) -> Result<()>
  where
    F: Fn(&Path, &str, String, bool, Instant, &TEnvironment) -> Result<()> + Send + 'static + Clone,
  {
    let file_text = FileText::new(environment.read_file(&file_path)?);

    if let Some(incremental_file) = incremental_file {
      if incremental_file.is_file_same(file_path, file_text.as_str()) {
        log_verbose!(environment, "No change: {}", file_path.display());
        return Ok(());
      }
    }

    let (start_instant, formatted_text) = {
      let start_instant = Instant::now();
      let mut file_text = Cow::Borrowed(file_text.as_str());
      let plugins_len = plugins.len();
      for (i, plugin) in plugins.iter_mut().enumerate() {
        let start_instant = Instant::now();
        let format_text_result = plugin
          .pool
          .format_measuring_time(|| plugin.plugin.format_text(file_path, &file_text, &ConfigKeyMap::new()));
        log_verbose!(
          environment,
          "Formatted file: {} in {}ms{}",
          file_path.display(),
          start_instant.elapsed().as_millis(),
          if plugins_len > 1 {
            format!(" (Plugin {}/{})", i + 1, plugins_len)
          } else {
            String::new()
          },
        );
        file_text = Cow::Owned(format_text_result?);
      }
      (start_instant, file_text.into_owned())
    };

    if let Some(incremental_file) = incremental_file {
      incremental_file.update_file(file_path, &formatted_text);
    }

    f(file_path, file_text.as_str(), formatted_text, file_text.has_bom(), start_instant, environment)?;

    Ok(())
  }
}
