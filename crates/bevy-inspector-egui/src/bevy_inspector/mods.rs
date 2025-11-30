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

pub type EntityComponentContextMenu<'f> = fn(
    &mut egui::Ui,
    Entity,
    &mut RestrictedWorldView<'_>, // component_view
    &mut Context<'_>,
    &TypeRegistry,
);

pub type EntitiesComponentContextMenu<'f> = fn(
    &mut egui::Ui,
    &[Entity],
    &mut RestrictedWorldView<'_>, // component_view
    &mut Context<'_>,
    &TypeRegistry,
);

pub fn ui_for_entity(
    world: &mut World,
    entity: Entity,
    ui: &mut egui::Ui,
    // BEGIN MOD - allow for custom context menu additions
    mod_context_menu: Option<EntityComponentContextMenu>,
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
        mod_context_menu,
    );
    queue.apply(world);
}

pub fn ui_for_entity_with_children(
    world: &mut World,
    entity: Entity,
    ui: &mut egui::Ui,
    // BEGIN MOD - allow for custom context menu additions
    mod_context_menu: Option<EntityComponentContextMenu>,
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
        mod_context_menu,
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
    mod_context_menu: Option<EntityComponentContextMenu>,
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
        mod_context_menu,
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
                        mod_context_menu,
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
    mod_context_menu: Option<EntityComponentContextMenu>,
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

        let response = header.show(ui, |ui| {
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

        // BEGIN MOD - allow user context menu
        response.header_response.context_menu(|ui| {
            if let Some(context_menu) = mod_context_menu {
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
            }
        });
        // END MOD

        #[cfg(feature = "documentation")]
        crate::egui_utils::show_docs(response.header_response, type_docs);

        ui.reset_style();
    }
}

pub fn ui_for_entities_shared_components(
    world: &mut World,
    entities: &[Entity],
    ui: &mut egui::Ui,
    // BEGIN MOD - allow for custom context menu additions
    mod_context_menu: Option<EntitiesComponentContextMenu>,
    // END MOD
) {
    let type_registry = world.resource::<AppTypeRegistry>().0.clone();
    let type_registry = type_registry.read();

    let Some(&first) = entities.first() else {
        return;
    };

    let Ok(mut components) = components_of_entity(&mut world.into(), first) else {
        return errors::entity_does_not_exist(ui, first);
    };

    for &entity in entities.iter().skip(1) {
        components.retain(|(_, id, _, _)| {
            world
                .get_entity(entity)
                .map_or(true, |entity| entity.contains_id(*id))
        })
    }

    let mut queue = CommandQueue::default();

    let id = egui::Id::NULL;
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

        let (resources_view, mut components_view) =
            RestrictedWorldView::resources_components(world);
        let mut cx = Context {
            world: Some(resources_view),
            queue: Some(&mut queue),
        };

        let mut values = Vec::with_capacity(entities.len());
        for (i, &entity) in entities.iter().enumerate() {
            // skip duplicate entities
            if entities[0..i].contains(&entity) {
                continue;
            };

            // SAFETY: entities are distinct, env has a context with just resources
            match unsafe {
                components_view.get_entity_component_reflect_unchecked(
                    entity,
                    component_type_id,
                    &type_registry,
                )
            } {
                Ok(value) => {
                    values.push(value);
                }
                Err(error) => {
                    errors::show_error(error, ui, &name);
                    return;
                }
            }
        }

        let response = header.show(ui, |ui| {
            ui.reset_style();

            let mut env = InspectorUi::for_bevy(&type_registry, &mut cx);
            let id = id.with(component_id);
            let options = &();

            let mut values_reflect: Vec<_> = values
                .iter_mut()
                .map(|value| value.bypass_change_detection().as_partial_reflect_mut())
                .collect();

            let changed = env.ui_for_reflect_many_with_options(
                component_type_id,
                &name,
                ui,
                id,
                options,
                values_reflect.as_mut_slice(),
                &|a| a,
            );

            if changed {
                for value in values.iter_mut() {
                    value.set_changed();
                }
            }
        });

        // BEGIN MOD - allow user context menu
        response.header_response.context_menu(|ui| {
            if let Some(context_menu) = mod_context_menu {
                (context_menu)(ui, entities, &mut components_view, &mut cx, &type_registry);
            }
        });
        // END MOD
    }

    queue.apply(world);
}
