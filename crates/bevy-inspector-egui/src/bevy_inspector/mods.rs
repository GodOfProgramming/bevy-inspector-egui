#[cfg(feature = "highlight_changes")]
use crate::bevy_inspector::set_highlight_style;
use crate::{
    bevy_inspector::{EntityFilter, Filter, components_of_entity, errors},
    reflect_inspector::{Context, InspectorUi},
    restricted_world_view::{ReflectBorrow, RestrictedWorldView},
    utils::{self, guess_entity_name::guess_entity_name},
};
use bevy_ecs::{
    change_detection::{DetectChanges, DetectChangesMut},
    entity::Entity,
    hierarchy::Children,
    reflect::AppTypeRegistry,
    world::{CommandQueue, World},
};
use bevy_reflect::TypeRegistry;
use std::path::Path;

pub type ComponentContextMenu<'f> = &'f mut dyn FnMut(
    &mut egui::Ui,
    Entity,
    &mut RestrictedWorldView<'_>, // component_view
    &mut Context<'_>,
    &TypeRegistry,
);

pub fn ui_for_entity(
    world: &mut World,
    entity: Entity,
    ui: &mut egui::Ui,
    // BEGIN MOD - allow for custom context menu additions
    mut mod_context_menu: Option<ComponentContextMenu>,
    // END MOD
) {
    let type_registry = world.resource::<AppTypeRegistry>().0.clone();
    let type_registry = type_registry.read();

    let entity_name = guess_entity_name(world, entity);
    ui.label(entity_name);

    let mut queue = CommandQueue::default();
    ui_for_entity_components(
        &mut world.into(),
        Some(&mut queue),
        entity,
        ui,
        egui::Id::new(entity),
        &type_registry,
        &mut mod_context_menu,
    );
    queue.apply(world);
}

pub fn ui_for_entity_with_children(
    world: &mut World,
    entity: Entity,
    ui: &mut egui::Ui,
    // BEGIN MOD - allow for custom context menu additions
    mut mod_context_menu: Option<ComponentContextMenu>,
    // END MOD
) {
    let type_registry = world.resource::<AppTypeRegistry>().0.clone();
    let type_registry = type_registry.read();

    let entity_name = guess_entity_name(world, entity);
    ui.label(entity_name);

    let filter: Filter = Filter::all();
    ui_for_entity_with_children_inner(
        world,
        entity,
        ui,
        egui::Id::new(entity),
        &type_registry,
        &filter,
        &mut mod_context_menu,
    )
}

pub fn ui_for_entity_with_children_inner<F>(
    world: &mut World,
    entity: Entity,
    ui: &mut egui::Ui,
    id: egui::Id,
    type_registry: &TypeRegistry,
    filter: &F,
    // BEGIN MOD - allow for custom context menu additions
    _mod_context_menu: &mut Option<ComponentContextMenu>,
    // END MOD
) where
    F: EntityFilter,
{
    let mut queue = CommandQueue::default();
    ui_for_entity_components(
        &mut world.into(),
        Some(&mut queue),
        entity,
        ui,
        id,
        type_registry,
        _mod_context_menu,
    );

    let children = world
        .get::<Children>(entity)
        .map(|children| children.iter().cloned().collect::<Vec<_>>());
    if let Some(mut children) = children
        && !children.is_empty()
    {
        filter.filter_entities(world, &mut children);
        ui.label("Children");
        for child in children {
            let id = id.with(child);

            let child_entity_name = guess_entity_name(world, child);
            egui::CollapsingHeader::new(&child_entity_name)
                .id_salt(id)
                .show(ui, |ui| {
                    ui.label(&child_entity_name);

                    ui_for_entity_with_children_inner(
                        world,
                        child,
                        ui,
                        id,
                        type_registry,
                        filter,
                        _mod_context_menu,
                    );
                });
        }
    }

    queue.apply(world);
}

pub fn ui_for_entity_components(
    world: &mut RestrictedWorldView<'_>,
    mut queue: Option<&mut CommandQueue>,
    entity: Entity,
    ui: &mut egui::Ui,
    id: egui::Id,
    type_registry: &TypeRegistry,
    // BEGIN MOD - allow for custom context menu additions
    mod_context_menu: &mut Option<ComponentContextMenu>,
    // END MOD
) {
    let Ok(components) = components_of_entity(world, entity) else {
        errors::entity_does_not_exist(ui, entity);
        return;
    };

    for (name, component_id, component_type_id, size) in components {
        let id = id.with(component_id);

        let header = egui::CollapsingHeader::new(&name).id_salt(id);

        let Some(component_type_id) = component_type_id else {
            header.show(ui, |ui| errors::no_type_id(ui, &name));
            continue;
        };

        #[cfg(feature = "documentation")]
        let type_docs = type_registry
            .get_type_info(component_type_id)
            .and_then(|info| info.docs());

        if size == 0 {
            ui.indent(id, |ui| {
                let _response = ui.label(&name);
                #[cfg(feature = "documentation")]
                crate::egui_utils::show_docs(_response, type_docs);
            });
            continue;
        }

        // create a context with access to the world except for the currently viewed component
        let (mut component_view, world) = world.split_off_component((entity, component_type_id));
        let mut cx = Context {
            world: Some(world),
            #[allow(clippy::needless_option_as_deref)]
            queue: queue.as_deref_mut(),
        };

        let value = match component_view.get_entity_component_reflect(
            entity,
            component_type_id,
            type_registry,
        ) {
            Ok(value) => value,
            Err(e) => {
                ui.indent(id, |ui| {
                    let response = ui.label(egui::RichText::new(&name).underline());
                    response.on_hover_ui(|ui| errors::show_error(e, ui, &name));
                });
                continue;
            }
        };

        let changed_by = match &value {
            ReflectBorrow::Mutable(val) => val.changed_by().into_option(),
            ReflectBorrow::Immutable(_) => None,
        };

        if value.is_changed() {
            #[cfg(feature = "highlight_changes")]
            set_highlight_style(ui);
        }

        let _response = header.show(ui, |ui| {
            ui.reset_style();

            let mut env = InspectorUi::for_bevy(type_registry, &mut cx);
            let id = id.with(component_id);
            let options = &();

            match value {
                ReflectBorrow::Mutable(mut value) => {
                    let changed = env.ui_for_reflect_with_options(
                        value.bypass_change_detection().as_partial_reflect_mut(),
                        ui,
                        id,
                        options,
                    );

                    if changed {
                        value.set_changed();
                    }
                }
                ReflectBorrow::Immutable(value) => env.ui_for_reflect_readonly_with_options(
                    value.as_partial_reflect(),
                    ui,
                    id,
                    options,
                ),
            };
        });

        let response = _response.header_response;

        // BEGIN MOD - allow user context menu
        if let Some(context_menu) = mod_context_menu {
            response.context_menu(|ui| {
                (context_menu)(ui, entity, &mut component_view, &mut cx, type_registry);

                if let Some(location) = changed_by {
                    ui.label("Last change:");
                    let path = Path::new(location.file());
                    let pretty = utils::trim_cargo_registry_path(path);

                    if ui
                        .button(format!(
                            "{}:{}:{}",
                            pretty.as_deref().unwrap_or(path).display(),
                            location.line(),
                            location.column()
                        ))
                        .clicked()
                    {
                        if let Err(e) = utils::open_file_at(location) {
                            bevy_log::error!("Failed to open last change location: {}", e);
                        } else {
                            bevy_log::info!("Successfully opened {location}");
                        }
                    }
                }
            });
        }
        // END MOD

        #[cfg(feature = "documentation")]
        crate::egui_utils::show_docs(response, type_docs);

        ui.reset_style();
    }
}
