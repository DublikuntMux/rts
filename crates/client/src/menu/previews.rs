//! Map previews for saves and replays whose map is not bundled, made once per map.

use std::collections::BTreeMap;

use bevy::prelude::*;
use rts_assets::Graphics;

use super::widgets::{EDGE, FitPreview};
use crate::{library::Record, preview::map_preview};

/// Generated previews by match setup checksum.
#[derive(Resource, Default)]
pub struct PreviewCache(BTreeMap<u64, Handle<Image>>);

/// A record's map image, generated on first use.
pub fn preview_for(record: &Record, aspect: f32) -> impl Scene {
    let setup = record.meta.setup.clone();
    bsn! {
        template(move |context| {
            let key = setup.checksum();
            if let Some(handle) = context.resource::<PreviewCache>().0.get(&key) {
                return Ok(ImageNode::new(handle.clone()));
            }
            // A record with a broken setup still shows its details; only the image is blank.
            let Ok(sim) = setup.simulation() else {
                return Ok(ImageNode::default());
            };
            let image = map_preview(&sim, context.resource::<Graphics>());
            let handle = context.resource_mut::<Assets<Image>>().add(image);
            context.resource_mut::<PreviewCache>().0.insert(key, handle.clone());
            Ok(ImageNode::new(handle))
        })
        FitPreview(aspect)
        Node {
            align_self: AlignSelf::Center,
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(4)),
        }
        BorderColor::all(EDGE)
    }
}
