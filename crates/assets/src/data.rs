//! Asset loaders for simulation data files.

use std::marker::PhantomData;

use bevy::{
    asset::{AssetLoader, LoadContext, io::Reader},
    prelude::*,
};
use rts_base::{LoadError, MapData, Rules};

/// A `*.rules.ron` file.
#[derive(Asset, TypePath)]
pub struct RulesAsset(pub Rules);

/// A `*.map.ron` file.
#[derive(Asset, TypePath)]
pub struct MapAsset(pub MapData);

pub struct DataPlugin;

impl Plugin for DataPlugin {
    fn build(&self, app: &mut App) {
        app.init_asset::<RulesAsset>()
            .init_asset::<MapAsset>()
            .register_asset_loader(RonLoader::<RulesAsset>(PhantomData))
            .register_asset_loader(RonLoader::<MapAsset>(PhantomData));
    }
}

/// An asset parsed from a RON file with a type-specific extension.
trait RonAsset: Asset + Sized {
    const EXTENSION: &'static str;

    fn parse(text: &str) -> Result<Self, LoadError>;
}

impl RonAsset for RulesAsset {
    const EXTENSION: &'static str = "rules.ron";

    fn parse(text: &str) -> Result<Self, LoadError> {
        Rules::from_ron(text).map(Self)
    }
}

impl RonAsset for MapAsset {
    const EXTENSION: &'static str = "map.ron";

    fn parse(text: &str) -> Result<Self, LoadError> {
        MapData::from_ron(text).map(Self)
    }
}

#[derive(TypePath)]
struct RonLoader<A>(PhantomData<A>);

impl<A: RonAsset> AssetLoader for RonLoader<A> {
    type Asset = A;
    type Settings = ();
    type Error = BevyError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        (): &(),
        _: &mut LoadContext<'_>,
    ) -> Result<A, BevyError> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;
        Ok(A::parse(str::from_utf8(&bytes)?)?)
    }

    fn extensions(&self) -> &[&str] {
        &[A::EXTENSION]
    }
}
