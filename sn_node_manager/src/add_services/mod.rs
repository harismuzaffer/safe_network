// Copyright (C) 2024 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.
pub mod config;
#[cfg(test)]
mod tests;

use self::config::{
    AddDaemonServiceOptions, AddFaucetServiceOptions, AddServiceOptions,
    InstallFaucetServiceCtxBuilder, InstallNodeServiceCtxBuilder,
};
use crate::{config::create_owned_dir, VerbosityLevel, DAEMON_SERVICE_NAME};
use color_eyre::{eyre::eyre, Help, Result};
use colored::Colorize;
use service_manager::ServiceInstallCtx;
use sn_service_management::{
    control::ServiceControl, DaemonServiceData, FaucetServiceData, NodeRegistry, NodeServiceData,
    ServiceStatus,
};
use std::{
    ffi::OsString,
    net::{IpAddr, Ipv4Addr, SocketAddr},
};

/// Install safenode as a service.
///
/// This only defines the service; it does not start it.
///
/// There are several arguments that probably seem like they could be handled within the function,
/// but they enable more controlled unit testing.
pub async fn add(
    options: AddServiceOptions,
    node_registry: &mut NodeRegistry,
    service_control: &dyn ServiceControl,
    verbosity: VerbosityLevel,
) -> Result<()> {
    if options.genesis {
        if let Some(count) = options.count {
            if count > 1 {
                return Err(eyre!("A genesis node can only be added as a single node"));
            }
        }

        let genesis_node = node_registry.nodes.iter().find(|n| n.genesis);
        if genesis_node.is_some() {
            return Err(eyre!("A genesis node already exists"));
        }
    }

    if options.count.is_some() && options.node_port.is_some() {
        let count = options.count.unwrap();
        if count > 1 {
            return Err(eyre!(
                "Custom node port can only be used when adding a single service"
            ));
        }
    }

    let safenode_file_name = options
        .safenode_bin_path
        .file_name()
        .ok_or_else(|| eyre!("Could not get filename from the safenode download path"))?
        .to_string_lossy()
        .to_string();

    //  store the bootstrap peers and the provided env variable.
    {
        let mut should_save = false;
        let new_bootstrap_peers: Vec<_> = options
            .bootstrap_peers
            .iter()
            .filter(|peer| !node_registry.bootstrap_peers.contains(peer))
            .collect();
        if !new_bootstrap_peers.is_empty() {
            node_registry
                .bootstrap_peers
                .extend(new_bootstrap_peers.into_iter().cloned());
            should_save = true;
        }

        if options.env_variables.is_some() {
            node_registry.environment_variables = options.env_variables.clone();
            should_save = true;
        }

        if should_save {
            node_registry.save()?;
        }
    }

    let mut added_service_data = vec![];
    let mut failed_service_data = vec![];

    let current_node_count = node_registry.nodes.len() as u16;
    let target_node_count = current_node_count + options.count.unwrap_or(1);

    let mut node_number = current_node_count + 1;
    while node_number <= target_node_count {
        let rpc_free_port = service_control.get_available_port()?;
        let rpc_socket_addr = if let Some(addr) = options.rpc_address {
            SocketAddr::new(IpAddr::V4(addr), rpc_free_port)
        } else {
            SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), rpc_free_port)
        };

        let service_name = format!("safenode{node_number}");
        let service_data_dir_path = options.service_data_dir_path.join(service_name.clone());
        let service_safenode_path = service_data_dir_path.join(safenode_file_name.clone());
        let service_log_dir_path = options.service_log_dir_path.join(service_name.clone());

        create_owned_dir(service_data_dir_path.clone(), &options.user)?;
        create_owned_dir(service_log_dir_path.clone(), &options.user)?;

        std::fs::copy(
            options.safenode_bin_path.clone(),
            service_safenode_path.clone(),
        )?;
        let install_ctx = InstallNodeServiceCtxBuilder {
            bootstrap_peers: options.bootstrap_peers.clone(),
            data_dir_path: service_data_dir_path.clone(),
            env_variables: options.env_variables.clone(),
            genesis: options.genesis,
            local: options.local,
            log_dir_path: service_log_dir_path.clone(),
            name: service_name.clone(),
            node_port: options.node_port,
            rpc_socket_addr,
            safenode_path: service_safenode_path.clone(),
            service_user: options.user.clone(),
        }
        .build()?;

        match service_control.install(install_ctx) {
            Ok(()) => {
                added_service_data.push((
                    service_name.clone(),
                    service_safenode_path.to_string_lossy().into_owned(),
                    service_data_dir_path.to_string_lossy().into_owned(),
                    service_log_dir_path.to_string_lossy().into_owned(),
                    rpc_socket_addr,
                ));

                node_registry.nodes.push(NodeServiceData {
                    genesis: options.genesis,
                    local: options.local,
                    service_name,
                    user: options.user.clone(),
                    number: node_number,
                    rpc_socket_addr,
                    version: options.version.clone(),
                    status: ServiceStatus::Added,
                    listen_addr: None,
                    pid: None,
                    peer_id: None,
                    log_dir_path: service_log_dir_path.clone(),
                    data_dir_path: service_data_dir_path.clone(),
                    safenode_path: service_safenode_path,
                    connected_peers: None,
                });
                // We save the node registry for each service because it's possible any number of
                // services could fail to be added.
                node_registry.save()?;
            }
            Err(e) => {
                failed_service_data.push((service_name.clone(), e.to_string()));
            }
        }

        node_number += 1;
    }

    std::fs::remove_file(options.safenode_bin_path)?;

    if !added_service_data.is_empty() {
        println!("Services Added:");
        for install in added_service_data.iter() {
            println!(" {} {}", "✓".green(), install.0);
            if verbosity != VerbosityLevel::Minimal {
                println!("    - Safenode path: {}", install.1);
                println!("    - Data path: {}", install.2);
                println!("    - Log path: {}", install.3);
                println!("    - RPC port: {}", install.4);
            }
        }
        println!("[!] Note: newly added services have not been started");
    }

    if !failed_service_data.is_empty() {
        println!("Failed to add {} service(s):", failed_service_data.len());
        for failed in failed_service_data.iter() {
            println!("{} {}: {}", "✕".red(), failed.0, failed.1);
        }
        return Err(eyre!("Failed to add one or more services")
            .suggestion("However, any services that were successfully added will be usable."));
    }

    Ok(())
}

/// Install the daemon as a service.
///
/// This only defines the service; it does not start it.
pub fn add_daemon(
    options: AddDaemonServiceOptions,
    node_registry: &mut NodeRegistry,
    service_control: &dyn ServiceControl,
) -> Result<()> {
    if node_registry.daemon.is_some() {
        return Err(eyre!("A safenodemand service has already been created"));
    }

    std::fs::copy(
        options.daemon_download_bin_path.clone(),
        options.daemon_install_bin_path.clone(),
    )?;

    let install_ctx = ServiceInstallCtx {
        label: DAEMON_SERVICE_NAME.parse()?,
        program: options.daemon_install_bin_path.clone(),
        args: vec![
            OsString::from("--port"),
            OsString::from(options.port.to_string()),
            OsString::from("--address"),
            OsString::from(options.address.to_string()),
        ],
        contents: None,
        username: None,
        working_directory: None,
        environment: None,
    };

    match service_control.install(install_ctx) {
        Ok(()) => {
            let daemon = DaemonServiceData {
                daemon_path: options.daemon_install_bin_path.clone(),
                endpoint: Some(SocketAddr::new(IpAddr::V4(options.address), options.port)),
                pid: None,
                service_name: DAEMON_SERVICE_NAME.to_string(),
                status: ServiceStatus::Added,
                version: options.version,
            };
            node_registry.daemon = Some(daemon);
            println!("Daemon service added {}", "✓".green());
            println!("[!] Note: the service has not been started");
            node_registry.save()?;
            std::fs::remove_file(options.daemon_download_bin_path)?;
            Ok(())
        }
        Err(e) => {
            println!("Failed to add daemon service: {e}");
            Err(e.into())
        }
    }
}

/// Install the faucet as a service.
///
/// This only defines the service; it does not start it.
///
/// There are several arguments that probably seem like they could be handled within the function,
/// but they enable more controlled unit testing.
pub fn add_faucet(
    install_options: AddFaucetServiceOptions,
    node_registry: &mut NodeRegistry,
    service_control: &dyn ServiceControl,
    verbosity: VerbosityLevel,
) -> Result<()> {
    if node_registry.faucet.is_some() {
        return Err(eyre!("A faucet service has already been created"));
    }

    create_owned_dir(
        install_options.service_log_dir_path.clone(),
        &install_options.user,
    )?;

    std::fs::copy(
        install_options.faucet_download_bin_path.clone(),
        install_options.faucet_install_bin_path.clone(),
    )?;

    let install_ctx = InstallFaucetServiceCtxBuilder {
        bootstrap_peers: install_options.bootstrap_peers.clone(),
        env_variables: install_options.env_variables.clone(),
        faucet_path: install_options.faucet_install_bin_path.clone(),
        local: install_options.local,
        log_dir_path: install_options.service_log_dir_path.clone(),
        name: "faucet".to_string(),
        service_user: install_options.user.clone(),
    }
    .build()?;

    match service_control.install(install_ctx) {
        Ok(()) => {
            node_registry.faucet = Some(FaucetServiceData {
                faucet_path: install_options.faucet_install_bin_path.clone(),
                local: false,
                log_dir_path: install_options.service_log_dir_path.clone(),
                pid: None,
                service_name: "faucet".to_string(),
                status: ServiceStatus::Added,
                user: install_options.user.clone(),
                version: install_options.version,
            });
            println!("Faucet service added {}", "✓".green());
            if verbosity != VerbosityLevel::Minimal {
                println!(
                    "  - Bin path: {}",
                    install_options.faucet_install_bin_path.to_string_lossy()
                );
                println!(
                    "  - Data path: {}",
                    install_options.service_data_dir_path.to_string_lossy()
                );
                println!(
                    "  - Log path: {}",
                    install_options.service_log_dir_path.to_string_lossy()
                );
            }
            println!("[!] Note: the service has not been started");
            std::fs::remove_file(install_options.faucet_download_bin_path)?;
            node_registry.save()?;
            Ok(())
        }
        Err(e) => {
            println!("Failed to add faucet service: {e}");
            Err(e.into())
        }
    }
}