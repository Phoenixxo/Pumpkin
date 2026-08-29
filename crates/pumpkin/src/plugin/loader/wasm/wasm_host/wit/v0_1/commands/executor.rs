use std::{collections::HashMap, sync::Arc};

use pumpkin_util::text::{
    TextComponent,
    color::{Color, NamedColor},
};

use crate::{
    command::{
        CommandExecutor,
        context::string_range::StringRange,
        dispatcher::CommandError,
        suggestion::{Suggestion, suggestions::Suggestions},
        tree::{CommandSuggestionProvider, CommandSuggestionResult},
    },
    plugin::loader::wasm::wasm_host::{
        DowncastResourceExt, PluginInstance, WasmPlugin,
        args::OwnedArg,
        state::{CommandSenderResource, ConsumedArgsResource, PluginHostState, ServerResource},
        wit::v0_1::pumpkin::plugin::command::{CommandError as CommandErrorWit, SuggestionRequest},
    },
    server::Server,
};

fn remove_resource<T: 'static>(state: &mut PluginHostState, rep: u32) {
    let _ = state
        .resource_table
        .delete::<T>(wasmtime::component::Resource::new_own(rep));
}

pub struct WasmCommandExecutor {
    pub handler_id: u32,
    pub plugin: Arc<WasmPlugin>,
    pub server: Arc<Server>,
}

impl CommandExecutor for WasmCommandExecutor {
    fn execute(
        &self,
        sender: &crate::command::CommandSender,
        _server: &crate::server::Server,
        args: &crate::command::args::ConsumedArgs,
    ) -> crate::command::CommandResult {
        let sender = sender.clone();
        let server = self.server.clone();
        let owned_args: HashMap<String, OwnedArg> = args
            .iter()
            .map(|(name, value)| (name.to_string(), OwnedArg::from_arg(value)))
            .collect();
        let handler_id = self.handler_id;
        let function = match self.plugin.plugin_instance.as_ref() {
            PluginInstance::V0_1(plugin) => plugin.func_handle_command(),
        };

        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                self.plugin
                    .store
                    .call_guest(move |mut guest| {
                        Box::pin(async move {
                            let (sender_resource, server_resource, args_resource, reps) = guest
                                .with(|mut store| {
                                    let sender_resource =
                                        store.data_mut().add_command_sender(sender)?;
                                    let sender_rep = sender_resource.rep();
                                    let server_resource = match store.data_mut().add_server(server)
                                    {
                                        Ok(resource) => resource,
                                        Err(error) => {
                                            remove_resource::<CommandSenderResource>(
                                                store.data_mut(),
                                                sender_rep,
                                            );
                                            return Err(error);
                                        }
                                    };
                                    let server_rep = server_resource.rep();
                                    let args_resource = match store
                                        .data_mut()
                                        .add_owned_consumed_args(owned_args)
                                    {
                                        Ok(resource) => resource,
                                        Err(error) => {
                                            remove_resource::<ServerResource>(
                                                store.data_mut(),
                                                server_rep,
                                            );
                                            remove_resource::<CommandSenderResource>(
                                                store.data_mut(),
                                                sender_rep,
                                            );
                                            return Err(error);
                                        }
                                    };
                                    let reps = (sender_rep, server_rep, args_resource.rep());
                                    Ok::<_, wasmtime::Error>((
                                        sender_resource,
                                        server_resource,
                                        args_resource,
                                        reps,
                                    ))
                                })?;

                            let result = guest
                                .call(
                                    function,
                                    (handler_id, sender_resource, server_resource, args_resource),
                                )
                                .await;

                            guest.with(|mut store| {
                                let result = result.map(|(result,)| match result {
                                    Ok(value) => Ok(value),
                                    Err(CommandErrorWit::InvalidConsumption(value)) => {
                                        Err(CommandError::InvalidConsumption(value))
                                    }
                                    Err(CommandErrorWit::InvalidRequirement) => {
                                        Err(CommandError::InvalidRequirement)
                                    }
                                    Err(CommandErrorWit::PermissionDenied) => {
                                        Err(CommandError::PermissionDenied)
                                    }
                                    Err(CommandErrorWit::CommandFailed(resource)) => {
                                        Err(CommandError::CommandFailed(
                                            resource.consume(store.data_mut()).provider,
                                        ))
                                    }
                                });
                                remove_resource::<CommandSenderResource>(store.data_mut(), reps.0);
                                remove_resource::<ServerResource>(store.data_mut(), reps.1);
                                remove_resource::<ConsumedArgsResource>(store.data_mut(), reps.2);
                                result
                            })
                        })
                    })
                    .await
                    .map_err(|error| {
                        CommandError::CommandFailed(
                            TextComponent::text(format!(
                                "Wasm command failed with following error: {error}"
                            ))
                            .color(Color::Named(NamedColor::Red)),
                        )
                    })?
            })
        })
    }
}

pub struct WasmCommandSuggestionProvider {
    pub handler_id: u32,
    pub plugin: Arc<WasmPlugin>,
    pub server: Arc<Server>,
}

impl CommandSuggestionProvider for WasmCommandSuggestionProvider {
    fn suggest(
        &self,
        src: &crate::command::CommandSender,
        _server: &Server,
        input: &str,
        start: usize,
        end: usize,
    ) -> CommandSuggestionResult {
        let request = SuggestionRequest {
            input: input.to_string(),
            cursor: input.len().try_into().unwrap_or(u32::MAX),
            start: start.try_into().unwrap_or(u32::MAX),
            remaining: input[start.min(input.len())..end.min(input.len())].to_string(),
        };
        let sender = src.clone();
        let server = self.server.clone();
        let handler_id = self.handler_id;
        let input_len = input.len();
        let function = match self.plugin.plugin_instance.as_ref() {
            PluginInstance::V0_1(plugin) => plugin.func_handle_command_suggestion(),
        };

        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                match self
                    .plugin
                    .store
                    .call_guest(move |mut guest| {
                        Box::pin(async move {
                            let (sender_resource, server_resource, reps) =
                                guest.with(|mut store| {
                                    let sender_resource =
                                        store.data_mut().add_command_sender(sender)?;
                                    let sender_rep = sender_resource.rep();
                                    let server_resource = match store.data_mut().add_server(server)
                                    {
                                        Ok(resource) => resource,
                                        Err(error) => {
                                            remove_resource::<CommandSenderResource>(
                                                store.data_mut(),
                                                sender_rep,
                                            );
                                            return Err(error);
                                        }
                                    };
                                    let reps = (sender_rep, server_resource.rep());
                                    Ok::<_, wasmtime::Error>((
                                        sender_resource,
                                        server_resource,
                                        reps,
                                    ))
                                })?;
                            let response = guest
                                .call(
                                    function,
                                    (handler_id, sender_resource, server_resource, request),
                                )
                                .await
                                .map(|(response,)| response);
                            guest.with(|mut store| {
                                let suggestions = response.map(|response| {
                                    let start = response.start as usize;
                                    let end = start.saturating_add(response.length as usize);
                                    let range = StringRange::between(start, end.min(input_len));
                                    let values = response
                                        .values
                                        .into_iter()
                                        .map(|suggestion| {
                                            if let Some(tooltip) = suggestion.tooltip {
                                                Suggestion::with_tooltip(
                                                    range,
                                                    suggestion.value,
                                                    tooltip.consume(store.data_mut()).provider,
                                                )
                                            } else {
                                                Suggestion::without_tooltip(range, suggestion.value)
                                            }
                                        })
                                        .collect();
                                    Suggestions::new(range, values)
                                });
                                remove_resource::<CommandSenderResource>(store.data_mut(), reps.0);
                                remove_resource::<ServerResource>(store.data_mut(), reps.1);
                                suggestions
                            })
                        })
                    })
                    .await
                {
                    Ok(suggestions) => suggestions,
                    Err(error) => {
                        tracing::error!("Wasm command suggestion failed: {error}");
                        Suggestions::empty()
                    }
                }
            })
        })
    }
}
