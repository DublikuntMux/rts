use bevy::prelude::*;

#[derive(Resource)]
pub struct GameFont(pub Handle<Font>);

impl FromWorld for GameFont {
    fn from_world(world: &mut World) -> Self {
        Self(world.load_asset("fonts/Exo2.ttf"))
    }
}
