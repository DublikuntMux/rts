Graphics format
===============

``assets/data/client.graphics.ron`` configures local presentation. It is
loaded and validated by ``rts_assets::Graphics`` before the lobby opens,
then kept across solo, online, and replay sessions. It does not travel in
match setup or replays and does not affect simulation hashes.

Edit the file and run ``cargo run`` to rebuild the asset pack. Settings are
read at startup; changing the source while the game runs requires a rebuild
and restart.

All fields are required; unknown fields are rejected. Colors are sRGB
``(red, green, blue)`` tuples with finite components from 0 to 1.

.. code-block:: rust

   (
       player_colors: [
           (0.20, 0.45, 0.90),
           (0.85, 0.20, 0.20),
       ],
       selection: (
           color: (0.95, 0.83, 0.45),
           padding: 4.0,
           accepted_order_color: (0.60, 0.90, 0.65),
           blocked_order_color: (1.0, 0.35, 0.25),
           marker_radius: 12.0,
           marker_cross_size: 5.0,
           marker_duration: 0.6,
       ),
       terrain_detail: (
           colors_below: 40.0,
           textures_at: 48.0,
       ),
   )

``player_colors`` must contain at least one color. Player slots index the
palette from zero and wrap when the palette is shorter than the player list.
The bundled palette has eight entries.

``selection.padding`` is the nonnegative distance in screen pixels between
the displayed unit footprint and its outline. ``selection.color`` also
colors the drag box. ``marker_radius`` and ``marker_cross_size`` are positive
screen pixel sizes for move feedback; ``marker_duration`` is a positive
number of seconds. All sizes and durations must be finite. Marker colors
distinguish accepted move targets from blocked terrain.

``terrain_detail`` sets the texture/color switch in screen pixels per tile.
Both thresholds must be finite and positive, and ``textures_at`` must exceed
``colors_below``. Textures remain visible at exactly ``colors_below``;
colors switch to textures at exactly ``textures_at``. See
:doc:`../architecture/rendering` for the hysteresis behavior.

Invalid or missing graphics data logs an asset loading error and keeps the
client in its loading state. There is no hardcoded graphics fallback.
Terrain colors, textures, unit radius, and unit visual bounds continue to
come from :doc:`rules-format`. Interface styling and camera controls are
defined in client code.
