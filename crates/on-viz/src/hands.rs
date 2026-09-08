//! Putting a downloaded hand model onto the keys.
//!
//! The hands are not modelled here. They are a premade, openly licensed, rigged glTF
//! that the user supplies, so nothing about the file can be assumed: not its units,
//! not which way it faces, not its bone names, and above all not its proportions.
//!
//! # Retargeting by joints, not by angles
//!
//! The obvious way to drive somebody else's rig is to copy joint angles onto it. That
//! does not work here, because the two skeletons do not share a rest pose or an axis
//! convention, and an angle only means something relative to those.
//!
//! The way that does work — and the only way a fingertip reliably lands on the right
//! key — is to place the rig's **joints**. Our own hand model has already worked out
//! where every knuckle, every interphalangeal joint and every fingertip has to be, in
//! millimetres of real piano. So each bone of the rig is moved so that its head sits
//! exactly on that joint and its length reaches exactly the next one. The rig's own
//! bone lengths stop mattering: whatever proportions the model was built with, the
//! posed result is the posture the biomechanics asked for.
//!
//! That is what fixes the two things a direction-only retarget gets wrong. Fingers no
//! longer cross over each other, because the metacarpals inside the palm are driven
//! too and the knuckles fan the way our skeleton says rather than the way the model's
//! rest pose happened to. And a fingertip lands on its key rather than near it,
//! because it is placed there rather than reached for.
//!
//! # One pass, top down
//!
//! Bone rotations are composed as the chain is walked, wrist outward, rather than
//! read back from the scene graph. Reading them back is a frame out of date — the
//! parent's transform is set by this very system — and over a four-bone chain that
//! stale rotation is exactly the sort of error that makes a hand shear when it moves
//! quickly.

use std::collections::HashMap;

use bevy::camera::visibility::NoFrustumCulling;
use bevy::mesh::skinning::SkinnedMesh;
use bevy::prelude::*;
use on_hand::skeleton::Joint;
use on_hand::{Finger, Hand};
use tracing::warn;

use crate::render::{Performance, Transport, KEYBOARD_Y_MM};
use crate::rig::{self, BoneRole};

// Kept, unworn. The gloves are not on the hands at the moment — a flat skin colour
// turned out to look better at the size the hands are actually drawn — but the glove
// is built and tested and this is what puts it back on.
#[allow(dead_code)]
/// How the hands are dressed.
///
/// In cartoon gloves: flat, saturated yellow with two dark marks on the back, and a
/// bare arm coming out of them. The shading is deliberately soft and almost matte —
/// what makes a drawn glove read as drawn is that it has one colour, not a highlight
/// travelling over it.
///
/// The colour comes from a texture painted onto the model when it loads, in the model's
/// own texture space; see [`crate::glove`] for why it has to be done that way.
fn wear_gloves(material: &mut StandardMaterial, cloth: Option<Handle<Image>>) {
    material.base_color = GLOVE_YELLOW;
    material.normal_map_texture = cloth;
    material.perceptual_roughness = 0.78;
    material.reflectance = 0.16;
    material.metallic = 0.0;
    // A skinned hand grazes itself where the thumb meets the palm, and a single-sided
    // surface shows that as a hole straight through the hand. Two extra triangles'
    // worth of fill on two hands is not worth arguing about.
    material.double_sided = true;
    material.cull_mode = None;
    // Cloth over a finger passes very little light, and a flat cartoon colour is spoilt
    // by glow coming through it.
    material.diffuse_transmission = 0.08;
    material.thickness = 8.0;
}

// Kept, unworn. The gloves are not on the hands at the moment — a flat skin colour
// turned out to look better at the size the hands are actually drawn — but the glove
// is built and tested and this is what puts it back on.
#[allow(dead_code)]
/// The gloves' own yellow, and the skin of the hand inside them.
///
/// The hand is left plain. It is a skeleton for the glove to be worn on and the only
/// part of it anyone sees is the arm, which is bare.
const GLOVE_YELLOW: Color = Color::srgb(0.97, 0.78, 0.11);
const BARE_SKIN: Color = Color::srgb(0.87, 0.69, 0.58);

/// Which bones belong to the arm rather than to the hand.
fn is_arm_bone(name: &str) -> bool {
    let lowered = name.to_ascii_lowercase();
    lowered.contains("lowerarm") || lowered.contains("forearm") || lowered.contains("upperarm")
}

/// One digit's bones in a loaded rig, ordered from the palm outward.
///
/// How many there are varies by rig and by digit. Four is the full chain — metacarpal,
/// proximal, intermediate, distal — and three is what most rigs give the thumb, whose
/// first bone *is* its metacarpal. Either way the last bone ends at the fingertip, and
/// that is what the chain is anchored from: the bones are matched to the last joints
/// of the digit, so however few there are, the tip lands on the key.
struct Digit {
    finger: Finger,
    /// Every bone of the digit, wrist end first.
    bones: Vec<DrivenBone>,
}

/// A bone of a loaded hand model that this app drives.
struct DrivenBone {
    entity: Entity,
    /// The rotation the model was exported with. Retargeting is composed onto it, so
    /// whatever roll the rig was authored with survives — which is what keeps the
    /// skinning from twisting.
    rest_rotation: Quat,
    /// Where the bone sits relative to its parent in the rest pose. Kept only by the
    /// metacarpals; the rest are moved onto joints.
    rest_translation: Vec3,
}

/// A loaded hand model, and everything needed to pose it.
#[derive(Component)]
pub struct HandRig {
    hand: Hand,
    /// Turns the model's own rest orientation into ours. Measured from the rig when
    /// it loads rather than assumed, so a model exported facing any direction works.
    correction: Quat,
    /// Where the model's wrist bone sits relative to the model's root, and how it is
    /// turned there. Fixed — the nodes between the two are scene structure that never
    /// moves — so it is composed once at load rather than read from a scene graph
    /// that is a frame behind.
    wrist_offset: Vec3,
    wrist_rotation: Quat,
    /// Scale that brings the model's palm to the size of the pianist's, so a model
    /// authored in metres, centimetres or arbitrary units all work.
    scale: f32,
    /// The digits, once the rig has been read.
    digits: Vec<Digit>,
    /// The forearm, if the model has one: the bone nearest the body, where it starts
    /// in the model, and the rest rotation it was authored with.
    forearm: Option<Forearm>,
}

/// The forearm the hand is on the end of.
///
/// The model is placed by putting its wrist where the animator says the wrist goes, and
/// everything above the wrist comes along for the ride. Left at that the arm is a rigid
/// stick pivoting about the wrist: turn the hand to reach a black key and the whole
/// forearm swings with it, which is exactly backwards — the arm leads and the hand
/// turns on the end of it.
///
/// So the forearm is aimed on its own, at a point below the keyboard where the player
/// would be sitting, and the model is then placed to put the wrist back where it
/// belongs given that aim.
#[derive(Clone)]
struct Forearm {
    /// The forearm bones, from the body outward.
    ///
    /// A MakeHuman rig splits the forearm in two so that the twist of pronation can be
    /// shared between them rather than wrung out of the skin at one joint. Turning only
    /// the first of them and leaving the second where it was authored is what put a
    /// crease across the wrist.
    bones: Vec<(Entity, Quat)>,
    /// Where the arm starts, in the model's own space.
    head: Vec3,
    /// The wrist bone and the rotation it was authored with.
    ///
    /// The hand is posed against the model's rest pose, so an arm that has swung round
    /// takes the wrist — and with it the palm — somewhere the fingers were not expecting.
    /// Undoing the swing here is what keeps the hand pointing where the hand model says
    /// and leaves the turn to the arm, where it belongs.
    wrist: Entity,
    wrist_rest: Quat,
}

/// How far below the keys the arms are drawn as hanging from.
///
/// Far enough to be off the bottom of the picture, because there is no torso to draw:
/// the arms run out of frame and the eye supplies the rest. This is the one thing about
/// the player that is not anatomical, and it is a framing decision rather than a claim —
/// a real shoulder is above the keys and behind them, which from this camera would put
/// the arms pointing at the viewer.
///
/// Everything else about where the player is comes from [`Torso`]: how far apart the
/// shoulders are, and how far they lean when a hand goes to the end of the keyboard. So
/// the arms hang from the same player the reach is measured from, and lean when that
/// player would have to.
const SHOULDER_BELOW_MM: f32 = 1_150.0;

/// Marks a loaded hand as still needing its bones identified.
#[derive(Component)]
struct NeedsRigging {
    hand: Hand,
}

/// A hand mesh that has been prepared: unculled, and wearing our own skin.
#[derive(Component)]
struct HandMesh;

/// The systems that load, rig and pose the hands.
pub struct HandsPlugin;

impl Plugin for HandsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_hands)
            .add_systems(Update, (rig_loaded_hands, prepare_hand_meshes));
    }
}

/// Load the hand models, if they are there.
fn setup_hands(mut commands: Commands, assets: Res<AssetServer>, performance: Res<Performance>) {
    if !performance.hands_available {
        warn!(
            "no rigged hand models found, so the keyboard will play without hands. \
             See assets/hands/README.md."
        );
        return;
    }
    for hand in Hand::ALL {
        // Relative to the asset root, which is the assets directory.
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

/// Everything found while walking a freshly loaded scene.
struct Survey {
    /// Every named node, with its parent, local transform and world position.
    nodes: HashMap<Entity, Node>,
    /// The bones this app recognises.
    roles: HashMap<BoneRole, Entity>,
    /// The bones that belong to the arm rather than to the hand.
    arms: std::collections::HashSet<Entity>,
}

/// One node of a loaded scene.
struct Node {
    parent: Option<Entity>,
    local: Transform,
    world: Vec3,
}

/// The forearm bone the model hangs from, if it has one.
///
/// The one wanted is the bone nearest the body whose parent is the model's root, so
/// that turning it turns the whole arm and the local rotation set on it is expressed in
/// the model's own space. A model with no forearm — which is what this used to export —
/// simply has none, and the arm is not posed.
fn find_forearm(survey: &Survey, root: Entity, wrist: Entity) -> Option<Forearm> {
    let (first, node) = survey
        .nodes
        .iter()
        .filter(|(_, node)| node.parent == Some(root))
        .find(|(entity, _)| survey.arms.contains(entity))?;

    // Every arm bone from there down to the wrist, in order. Walking down from the
    // shoulder end would need to know which child to follow at each step; walking up
    // from the wrist cannot go wrong, since a bone has only one parent.
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

/// Once a hand model has loaded, work out how to drive it.
///
/// Everything is measured rather than assumed: which way the model faces, where its
/// wrist is, how big it is, and which bones make up each digit. That is what lets a
/// different rigged hand be dropped in without touching any code.
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
        // Measured inside the root's own frame: the root has already been placed and
        // turned this frame, and folding that into the measurement would apply it
        // twice.
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

/// Walk a loaded scene, recording every node and the bones we recognise.
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
    // The scene spawns asynchronously; on the frame the root exists but its children
    // do not, there is nothing to measure yet and it is worth waiting a frame.
    (!survey.roles.is_empty()).then_some(survey)
}

/// Assemble each digit's chain of bones, wrist end first.
///
/// The metacarpals are found by walking *up* from the proximal phalanx rather than by
/// name. Rigs disagree wildly about what to call a metacarpal — or leave it out — but
/// they all agree about where it is: between the wrist and the knuckle.
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

        // Anything between the wrist and the knuckle is palm: a metacarpal, or a
        // chain of them. Only the last one matters, since it is the one that reaches
        // the knuckle; any before it stay where the model put them.
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
        // If the walk never reached the wrist the chain is not what we think it is,
        // and driving it would be guesswork.
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

/// How a loaded rig has to be turned, moved and scaled to become our hand.
struct RigFit {
    correction: Quat,
    wrist_offset: Vec3,
    wrist_rotation: Quat,
    scale: f32,
}

/// Measure how a loaded rig is built, from the rest pose it arrived in.
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

    // Scale from the palm rather than from the whole hand. Every joint beyond the
    // knuckles is placed explicitly, so their spacing in the model is irrelevant;
    // what the scale has to get right is how big the *mesh* is — how wide the palm
    // and how thick the fingers — and the palm is what says that.
    let measured = (middle_knuckle - wrist).length();
    let skeleton = performance.animators[hand as usize].skeleton();
    let wanted = {
        let rest = skeleton.rest_pose(glam::Vec3::ZERO);
        let posture = skeleton.forward(&rest);
        (posture.joint(Joint::Base(Finger::Middle)) - posture.wrist).length()
    };
    let scale = if measured > 1e-6 { wanted / measured } else { 1.0 };

    // The wrist bone's own place in the model, so the root can be positioned to bring
    // it exactly where the hand model says the wrist is.
    let wrist_entity = *survey.roles.get(&BoneRole::Wrist)?;
    let mut wrist_offset = Vec3::ZERO;
    let mut wrist_rotation = Quat::IDENTITY;
    // Up the chain until it runs out of surveyed nodes, which is the model's root —
    // the entity this app spawned, which has no name and so was never surveyed.
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

/// Finish preparing a hand mesh once the loader has produced one.
///
/// Culling is switched off. A skinned mesh keeps the bounding box it was authored
/// with, and a hand rig taken from a whole figure has that box wherever the hand hung
/// on the body — nowhere near where the hand is actually posed. Bevy would cull a mesh
/// that is plainly on screen, and recomputing the box every frame would cost more than
/// it saves for two hands.
///
/// The glove is painted here too, once, from the mesh's rest pose and the places its
/// own joints sit. Both are only available together on a mesh that has finished
/// loading, which is what this system is waiting for anyway.
///
/// And the hands are told not to *receive* shadows, though they still cast them. A
/// hand is a curved surface lit from almost straight above, so parts of it face the
/// light nearly edge-on, and a shadow map cannot resolve depth there: the palm ends up
/// shadowing itself in a hard-edged mess. Nothing is ever above a hand to cast a real
/// shadow onto it, so refusing them costs nothing and the acne goes with it. The
/// shadow the hand throws down onto the keys — the one cue that says how far off the
/// keys a finger is — is unaffected.
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


/// A plain hand: one skin colour, and nothing written into it.
///
/// Skin detail was tried and so were gloves. Both were the same mistake in different
/// clothes — the hand is perhaps two hundred pixels across in the finished picture, and
/// anything drawn into it at that size reads as noise rather than as detail. A flat
/// colour under the scene's own lighting is what actually looks like a hand here, and
/// the shading does the rest.
///
/// The glove is left in [`crate::glove`], built and tested but not worn.
fn bare_hand() -> StandardMaterial {
    StandardMaterial {
        base_color: BARE_SKIN,
        perceptual_roughness: 0.62,
        reflectance: 0.14,
        // A skinned hand grazes itself where the thumb meets the palm, and a
        // single-sided surface shows that as a hole straight through the hand.
        double_sided: true,
        cull_mode: None,
        ..default()
    }
}

// Kept, unworn. The gloves are not on the hands at the moment — a flat skin colour
// turned out to look better at the size the hands are actually drawn — but the glove
// is built and tested and this is what puts it back on.
#[allow(dead_code)]
/// Work out a hand's own frame from where its joints sit in the rest pose.
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

/// Pose the hands.
///
/// The posture comes from the animator, which is running the same biomechanical model
/// that chose the fingering, so what is on screen is a picture of the decision rather
/// than an illustration of it.
pub fn pose_hands(
    transport: Res<Transport>,
    performance: Res<Performance>,
    rigs: Query<(Entity, &HandRig)>,
    mut transforms: Query<&mut Transform>,
) {
    // The view puts the keyboard somewhere else than the hand model measures it.
    let offset = Vec3::Y * KEYBOARD_Y_MM;

    for (root, rig) in &rigs {
        if rig.digits.is_empty() {
            continue;
        }
        let animator = &performance.animators[rig.hand as usize];
        let pose = animator.pose_at(transport.position);
        let skeleton = animator.skeleton();
        let posture = skeleton.forward(&pose);

        // Place the whole model so its wrist bone lands where the hand model says the
        // wrist is, turned the way the hand model says it is turned.
        let root_rotation = skeleton.wrist_rotation(&pose) * rig.correction;
        let wrist_world = posture.wrist + offset;

        // Aim the forearm first, because doing so moves the wrist within the model and
        // the model then has to be placed to put it back. The arm points at where the
        // player's shoulder would be — below the keys and drawn in towards the middle —
        // so it stays hanging from the body while the hand turns on the end of it,
        // instead of swinging round rigidly every time the hand does.
        let mut wrist_in_model = rig.wrist_offset;
        if let Some(arm) = rig.forearm.as_ref() {
            // Where the arm points is a consequence of where the hand has gone, which
            // is what an arm hanging off a seated body does. The player leans towards a
            // hand that has gone to the end of the keyboard, and stops leaning when
            // their spine runs out, which is what stops the two arms crossing over each
            // other at the extremes.
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
                // The turn is worked out in the model's own space, which is where these
                // bones' local rotations are expressed.
                let turn = rig::align(rest_direction, root_rotation.inverse() * wanted);
                wrist_in_model = arm.head + turn * (rig.wrist_offset - arm.head);

                // Shared equally along the forearm rather than spent at its first
                // joint, so the skin winds gradually the way a real forearm does
                // instead of creasing where the one turned bone meets the ones that
                // did not.
                let share = 1.0 / arm.bones.len() as f32;
                let each = Quat::IDENTITY.slerp(turn, share);
                for (bone, rest) in &arm.bones {
                    if let Ok(mut transform) = transforms.get_mut(*bone) {
                        transform.rotation = each * *rest;
                    }
                }

                // And undone at the wrist. The hand is posed against the model's rest
                // pose, so without this the arm takes the palm round with it and leaves
                // the fingers pointing somewhere else — which is the seam.
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

/// Place one digit's bones on the joints our own skeleton computed.
///
/// The chain is anchored at the fingertip rather than at the wrist. Whatever bones a
/// rig gives a digit, the last one ends at the tip, the one before it at the joint
/// before that, and so on back toward the palm; only the very first bone keeps the
/// position the model gave it. Anchoring the other way round would put the fingertip
/// wherever the rig's own bone lengths happened to reach, which is the whole problem
/// this is here to solve — and it would drag the palm out of shape on any digit whose
/// first bone is already inside it.
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
    // Every joint of the digit, palm outward. The last is the fleshy tip that
    // touches the key.
    let joints = [
        posture.joint(Joint::Base(finger)) + offset,
        posture.joint(Joint::Middle(finger)) + offset,
        posture.joint(Joint::Distal(finger)) + offset,
        posture.joint(Joint::Tip(finger)) + offset,
    ];
    // Match the bones to the *last* joints, so the tip is always accounted for.
    let Some(first) = joints.len().checked_sub(digit.bones.len()) else {
        return;
    };
    let aims = &joints[first..];

    let mut parent_rotation = wrist_rotation;
    let mut parent_position = wrist_world;

    for (index, bone) in digit.bones.iter().enumerate() {
        // The first bone stays where the model put it — it is inside the palm, and
        // moving it would distort the palm rather than the finger. Every bone after
        // it starts on the joint the bone before it reached.
        let head = if index == 0 {
            parent_position + parent_rotation * (bone.rest_translation * scale)
        } else {
            aims[index - 1]
        };
        let aim = aims[index];

        let Ok(mut transform) = transforms.get_mut(bone.entity) else {
            continue;
        };
        // Expressed in the parent's frame, which is why the walk has to be top down:
        // the parent's rotation is set by this same loop, one bone earlier.
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
