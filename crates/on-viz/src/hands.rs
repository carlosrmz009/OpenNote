use std::collections::HashMap;

use bevy::camera::visibility::NoFrustumCulling;
use bevy::mesh::skinning::SkinnedMesh;
use bevy::prelude::*;
use on_hand::skeleton::Joint;
use on_hand::{Finger, Hand};
use tracing::warn;

use crate::render::{Performance, Transport, KEYBOARD_Y_MM};
use crate::rig::{self, BoneRole};

#[allow(dead_code)]
fn wear_gloves(material: &mut StandardMaterial, cloth: Option<Handle<Image>>) {
    material.base_color = GLOVE_YELLOW;
    material.normal_map_texture = cloth;
    material.perceptual_roughness = 0.78;
    material.reflectance = 0.16;
    material.metallic = 0.0;
    material.double_sided = true;
    material.cull_mode = None;
    material.diffuse_transmission = 0.08;
    material.thickness = 8.0;
}

#[allow(dead_code)]
const GLOVE_YELLOW: Color = Color::srgb(0.97, 0.78, 0.11);
const BARE_SKIN: Color = Color::srgb(0.87, 0.69, 0.58);

fn is_arm_bone(name: &str) -> bool {
    let lowered = name.to_ascii_lowercase();
    lowered.contains("lowerarm") || lowered.contains("forearm") || lowered.contains("upperarm")
}

struct Digit {
    finger: Finger,
    bones: Vec<DrivenBone>,
}

struct DrivenBone {
    entity: Entity,
    rest_rotation: Quat,
    rest_translation: Vec3,
}

#[derive(Component)]
pub struct HandRig {
    hand: Hand,
    correction: Quat,
    wrist_offset: Vec3,
    wrist_rotation: Quat,
    scale: f32,
    digits: Vec<Digit>,
    forearm: Option<Forearm>,
}

#[derive(Clone)]
struct Forearm {
    bones: Vec<(Entity, Quat)>,
    head: Vec3,
    wrist: Entity,
    wrist_rest: Quat,
}

const SHOULDER_BELOW_MM: f32 = 1_150.0;

#[derive(Component)]
struct NeedsRigging {
    hand: Hand,
}

#[derive(Component)]
struct HandMesh;

pub struct HandsPlugin;

impl Plugin for HandsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_hands)
            .add_systems(Update, (rig_loaded_hands, prepare_hand_meshes));
    }
}

fn setup_hands(mut commands: Commands, assets: Res<AssetServer>, performance: Res<Performance>) {
    if !performance.hands_available {
        warn!(
            "no rigged hand models found, so the keyboard will play without hands. \
             See assets/hands/README.md."
        );
        return;
    }
    for hand in Hand::ALL {
        let file = match hand {
            Hand::Left => "hands/hand-left.glb",
            Hand::Right => "hands/hand-right.glb",
        };
        let scene = assets.load(GltfAssetLabel::Scene(0).from_asset(file));
        commands.spawn((
            WorldAssetRoot(scene),
            Transform::IDENTITY,
            HandRig {
                hand,
                correction: Quat::IDENTITY,
                wrist_offset: Vec3::ZERO,
                wrist_rotation: Quat::IDENTITY,
                scale: 1.0,
                digits: Vec::new(),
                forearm: None,
            },
            NeedsRigging { hand },
        ));
    }
}

struct Survey {
    nodes: HashMap<Entity, Node>,
    roles: HashMap<BoneRole, Entity>,
    arms: std::collections::HashSet<Entity>,
}

struct Node {
    parent: Option<Entity>,
    local: Transform,
    world: Vec3,
}

fn find_forearm(survey: &Survey, root: Entity, wrist: Entity) -> Option<Forearm> {
    let (first, node) = survey
        .nodes
        .iter()
        .filter(|(_, node)| node.parent == Some(root))
        .find(|(entity, _)| survey.arms.contains(entity))?;

    let mut chain = Vec::new();
    let mut at = survey.nodes.get(&wrist)?.parent;
    while let Some(entity) = at {
        if !survey.arms.contains(&entity) {
            break;
        }
        let node = survey.nodes.get(&entity)?;
        chain.push((entity, node.local.rotation));
        at = node.parent;
    }
    chain.reverse();
    if chain.is_empty() {
        chain.push((*first, node.local.rotation));
    }

    Some(Forearm {
        bones: chain,
        head: node.world,
        wrist,
        wrist_rest: survey.nodes.get(&wrist)?.local.rotation,
    })
}

fn rig_loaded_hands(
    mut commands: Commands,
    performance: Res<Performance>,
    pending: Query<(Entity, &NeedsRigging)>,
    names: Query<(&Name, &Transform)>,
    parents: Query<&ChildOf>,
    globals: Query<&GlobalTransform>,
    children: Query<&Children>,
    mut rigs: Query<&mut HandRig>,
) {
    for (root, needs) in &pending {
        let Some(survey) = survey_scene(root, needs.hand, &children, &names, &parents, &globals)
        else {
            continue;
        };
        let Some(wrist) = survey.roles.get(&BoneRole::Wrist).copied() else {
            warn!("the {:?} hand rig has no wrist bone", needs.hand);
            commands.entity(root).remove::<NeedsRigging>();
            continue;
        };

        let Ok(root_global) = globals.get(root) else {
            continue;
        };
        let inverse = root_global.affine().inverse();
        let local = |entity: Entity| {
            survey
                .nodes
                .get(&entity)
                .map(|node| inverse.transform_point3(node.world))
        };

        let Some(fit) = measure_fit(needs.hand, &survey, &local, &performance) else {
            commands.entity(root).remove::<NeedsRigging>();
            continue;
        };
        let digits = build_digits(&survey, wrist);
        if digits.is_empty() {
            warn!("the {:?} hand rig has no finger bones", needs.hand);
            commands.entity(root).remove::<NeedsRigging>();
            continue;
        }

        let found: usize = digits.iter().map(|d| d.bones.len()).sum();
        debug!(
            "the {:?} hand rig has {} digits and {found} driven bones, scale {:.1}",
            needs.hand,
            digits.len(),
            fit.scale
        );

        if let Ok(mut rig) = rigs.get_mut(root) {
            rig.correction = fit.correction;
            rig.wrist_offset = fit.wrist_offset;
            rig.wrist_rotation = fit.wrist_rotation;
            rig.scale = fit.scale;
            rig.digits = digits;
            rig.forearm = find_forearm(&survey, root, wrist);
        }
        commands.entity(root).remove::<NeedsRigging>();
    }
}

fn survey_scene(
    root: Entity,
    hand: Hand,
    children: &Query<&Children>,
    names: &Query<(&Name, &Transform)>,
    parents: &Query<&ChildOf>,
    globals: &Query<&GlobalTransform>,
) -> Option<Survey> {
    let mut survey = Survey {
        nodes: HashMap::new(),
        roles: HashMap::new(),
        arms: std::collections::HashSet::new(),
    };
    let mut stack = vec![root];
    while let Some(entity) = stack.pop() {
        if let Ok(kids) = children.get(entity) {
            stack.extend(kids.iter());
        }
        let Ok((name, local)) = names.get(entity) else {
            continue;
        };
        let Ok(global) = globals.get(entity) else {
            continue;
        };
        survey.nodes.insert(
            entity,
            Node {
                parent: parents.get(entity).ok().map(|p| p.parent()),
                local: *local,
                world: global.translation(),
            },
        );
        if is_arm_bone(name.as_str()) {
            survey.arms.insert(entity);
        }
        if let Some(bone) = rig::classify(name.as_str()) {
            if bone.hand == hand {
                survey.roles.insert(bone.role, entity);
            }
        }
    }
    (!survey.roles.is_empty()).then_some(survey)
}

fn build_digits(survey: &Survey, wrist: Entity) -> Vec<Digit> {
    let mut digits = Vec::new();
    for finger in Finger::ALL {
        let (Some(&proximal), Some(&intermediate), Some(&distal)) = (
            survey.roles.get(&BoneRole::Proximal(finger)),
            survey.roles.get(&BoneRole::Intermediate(finger)),
            survey.roles.get(&BoneRole::Distal(finger)),
        ) else {
            continue;
        };

        let mut metacarpal = None;
        let mut walker = survey.nodes.get(&proximal).and_then(|node| node.parent);
        while let Some(entity) = walker {
            if entity == wrist {
                break;
            }
            if metacarpal.is_none() {
                metacarpal = Some(entity);
            }
            walker = survey.nodes.get(&entity).and_then(|node| node.parent);
        }
        if walker.is_none() {
            continue;
        }

        let mut bones = Vec::with_capacity(4);
        for entity in metacarpal
            .into_iter()
            .chain([proximal, intermediate, distal])
        {
            let Some(node) = survey.nodes.get(&entity) else {
                continue;
            };
            bones.push(DrivenBone {
                entity,
                rest_rotation: node.local.rotation,
                rest_translation: node.local.translation,
            });
        }
        digits.push(Digit { finger, bones });
    }
    digits
}

struct RigFit {
    correction: Quat,
    wrist_offset: Vec3,
    wrist_rotation: Quat,
    scale: f32,
}

fn measure_fit(
    hand: Hand,
    survey: &Survey,
    local: &impl Fn(Entity) -> Option<Vec3>,
    performance: &Performance,
) -> Option<RigFit> {
    let at = |role: BoneRole| survey.roles.get(&role).copied().and_then(local);

    let (Some(wrist), Some(tip), Some(index), Some(little), Some(middle_knuckle)) = (
        at(BoneRole::Wrist),
        at(BoneRole::Distal(Finger::Middle)),
        at(BoneRole::Proximal(Finger::Index)),
        at(BoneRole::Proximal(Finger::Little)),
        at(BoneRole::Proximal(Finger::Middle)),
    ) else {
        warn!("the {hand:?} hand rig is missing the bones needed to orient it");
        return None;
    };
    let correction = rig::rest_correction(hand, wrist, tip, index, little)?;

    let measured = (middle_knuckle - wrist).length();
    let skeleton = performance.animators[hand as usize].skeleton();
    let wanted = {
        let rest = skeleton.rest_pose(glam::Vec3::ZERO);
        let posture = skeleton.forward(&rest);
        (posture.joint(Joint::Base(Finger::Middle)) - posture.wrist).length()
    };
    let scale = if measured > 1e-6 { wanted / measured } else { 1.0 };

    let wrist_entity = *survey.roles.get(&BoneRole::Wrist)?;
    let mut wrist_offset = Vec3::ZERO;
    let mut wrist_rotation = Quat::IDENTITY;
    let mut walker = Some(wrist_entity);
    while let Some(node) = walker.and_then(|entity| survey.nodes.get(&entity)) {
        wrist_offset = node.local.transform_point(wrist_offset);
        wrist_rotation = node.local.rotation * wrist_rotation;
        walker = node.parent;
    }

    Some(RigFit {
        correction,
        wrist_offset,
        wrist_rotation,
        scale,
    })
}

fn prepare_hand_meshes(
    mut commands: Commands,
    mut materials: ResMut<Assets<StandardMaterial>>,
    skinned: Query<Entity, (With<Mesh3d>, With<SkinnedMesh>, Without<HandMesh>)>,
) {
    for entity in &skinned {
        commands.entity(entity).insert((
            HandMesh,
            NoFrustumCulling,
            bevy::light::NotShadowReceiver,
            MeshMaterial3d(materials.add(bare_hand())),
        ));
    }
}

fn bare_hand() -> StandardMaterial {
    StandardMaterial {
        base_color: BARE_SKIN,
        perceptual_roughness: 0.62,
        reflectance: 0.14,
        double_sided: true,
        cull_mode: None,
        ..default()
    }
}

#[allow(dead_code)]
fn hand_frame(
    joints: &[Entity],
    names: &Query<&Name>,
    places: &Query<&GlobalTransform>,
) -> Option<crate::glove::HandFrame> {
    let find = |wanted: &str| {
        joints
            .iter()
            .find(|joint| {
                names
                    .get(**joint)
                    .is_ok_and(|name| name.as_str().to_ascii_lowercase().contains(wanted))
            })
            .and_then(|joint| places.get(*joint).ok())
            .map(|place| place.translation())
    };
    Some(crate::glove::HandFrame::new(
        find("wrist")?,
        find("finger3-1")?,
        find("finger2-1")?,
        find("finger5-1")?,
        find("finger1-3")?,
    ))
}

pub fn pose_hands(
    transport: Res<Transport>,
    performance: Res<Performance>,
    rigs: Query<(Entity, &HandRig)>,
    mut transforms: Query<&mut Transform>,
) {
    let offset = Vec3::Y * KEYBOARD_Y_MM;
    let poses = crate::timeline::pose_both_decided(
        &performance.animators,
        &performance.decisions,
        transport.position,
    );

    for (root, rig) in &rigs {
        if rig.digits.is_empty() {
            continue;
        }
        let animator = &performance.animators[rig.hand as usize];
        let pose = poses[rig.hand as usize];
        let skeleton = animator.skeleton();
        let posture = skeleton.forward(&pose);

        let root_rotation = skeleton.wrist_rotation(&pose) * rig.correction;
        let wrist_world = posture.wrist + offset;

        let mut wrist_in_model = rig.wrist_offset;
        if let Some(arm) = rig.forearm.as_ref() {
            let torso = &performance.torso;
            let lean = torso.lean_for(rig.hand, wrist_world);
            let shoulder = Vec3::new(
                torso.shoulder(rig.hand, lean).x,
                -SHOULDER_BELOW_MM,
                wrist_world.z,
            );

            let wanted = (wrist_world - shoulder).normalize_or_zero();
            let rest_direction = (rig.wrist_offset - arm.head).normalize_or_zero();
            if wanted != Vec3::ZERO && rest_direction != Vec3::ZERO {
                let turn = rig::align(rest_direction, root_rotation.inverse() * wanted);
                wrist_in_model = arm.head + turn * (rig.wrist_offset - arm.head);

                let share = 1.0 / arm.bones.len() as f32;
                let each = Quat::IDENTITY.slerp(turn, share);
                for (bone, rest) in &arm.bones {
                    if let Ok(mut transform) = transforms.get_mut(*bone) {
                        transform.rotation = each * *rest;
                    }
                }

                if let Ok(mut transform) = transforms.get_mut(arm.wrist) {
                    transform.rotation = turn.inverse() * arm.wrist_rest;
                }
            }
        }

        let root_translation = wrist_world - root_rotation * (wrist_in_model * rig.scale);
        {
            let Ok(mut transform) = transforms.get_mut(root) else {
                continue;
            };
            transform.rotation = root_rotation;
            transform.scale = Vec3::splat(rig.scale);
            transform.translation = root_translation;
        }
        let wrist_rotation = root_rotation * rig.wrist_rotation;

        for digit in &rig.digits {
            pose_digit(
                digit,
                &posture,
                offset,
                wrist_world,
                wrist_rotation,
                rig.scale,
                &mut transforms,
            );
        }
    }
}

fn pose_digit(
    digit: &Digit,
    posture: &on_hand::skeleton::Posture,
    offset: Vec3,
    wrist_world: Vec3,
    wrist_rotation: Quat,
    scale: f32,
    transforms: &mut Query<&mut Transform>,
) {
    let finger = digit.finger;
    let joints = [
        posture.joint(Joint::Base(finger)) + offset,
        posture.joint(Joint::Middle(finger)) + offset,
        posture.joint(Joint::Distal(finger)) + offset,
        posture.joint(Joint::Tip(finger)) + offset,
    ];
    let Some(first) = joints.len().checked_sub(digit.bones.len()) else {
        return;
    };
    let aims = &joints[first..];

    let mut parent_rotation = wrist_rotation;
    let mut parent_position = wrist_world;

    for (index, bone) in digit.bones.iter().enumerate() {
        let head = if index == 0 {
            parent_position + parent_rotation * (bone.rest_translation * scale)
        } else {
            aims[index - 1]
        };
        let aim = aims[index];

        let Ok(mut transform) = transforms.get_mut(bone.entity) else {
            continue;
        };
        let inverse_parent = parent_rotation.inverse();
        transform.translation = (inverse_parent * (head - parent_position)) / scale;

        let wanted = (aim - head).normalize_or_zero();
        if wanted != Vec3::ZERO {
            let local = inverse_parent * wanted;
            let rest_direction = bone.rest_rotation * Vec3::Y;
            transform.rotation = rig::align(rest_direction, local) * bone.rest_rotation;
        }

        parent_rotation *= transform.rotation;
        parent_position = head;
    }
}
