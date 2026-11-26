Binary setup frame
==================

The server sends ``ServerMessage::Setup(LobbySetup)`` as a compact binary
message on ``STREAM_GAME`` (tag ``0``, raw-deflate compressed via header bit
``0x80`` whenever compression shrinks it). The codec lives in
``crates/net/src/wire.rs``.

The setup is public. It never contains the map's terrain, units, deposits,
starts, or the match seed. Each player learns the map only from its own
view frames; see :doc:`../architecture/visibility`.

Layout
------

Inside the binary wire envelope (header byte ``0x00`` uncompressed or ``0x80``
raw-deflate compressed), the ``Setup`` payload contains:

.. list-table:: Setup payload fields
   :header-rows: 1
   :widths: 22 22 56

   * - Field
     - Encoding
     - Meaning
   * - ``version``
     - LEB128 ``u32``
     - Protocol version, currently 10.
   * - ``players``
     - ``u8``
     - Player count, 1 to 8.
   * - ``map``
     - Length-prefixed UTF-8
     - Map display name (at most 64 chars, no controls).
   * - ``rules``
     - Length-prefixed UTF-8
     - Lexically minified rules text (at most 4 MiB).

Encoding
--------

The server encodes the rules as follows:

1. Checks that the original rules text is at most 4 MiB.
2. Minifies it lexically: removes comments and whitespace that does not
   separate tokens. String, character, raw-string, and byte literals, numbers,
   identifiers, and attributes keep their exact spelling.
3. Writes the payload and compresses it with raw DEFLATE level 6 via
   ``miniz_oxide`` whenever compression reduces the byte count.

The minifier never parses and reserializes numbers. Decimal speeds and
distances are converted to world units by every peer from the same spelling.
Minifying is idempotent; malformed input is copied through and rejected later
by validation.

Decoding
--------

A client accepts the frame only if every check passes, before it sends
``Loaded { checksum }``:

1. The wire frame does not exceed ``MAX_SERVER_MESSAGE_BYTES``.
2. When bit ``0x80`` is set, raw DEFLATE inflation is bounded before
   allocating beyond the limit and rejects truncated, trailing, or corrupt
   streams.
3. The protocol version matches (currently 11) and player count is 1 to 8.
4. Both ``map`` and ``rules`` are valid UTF-8 within their length bounds.
5. The rules parse and validate.

Checksum
--------

``LobbySetup::checksum`` is the 64-bit FNV-1a hash of, in order:

.. list-table::
   :header-rows: 1
   :widths: 40 60

   * - Bytes
     - Encoding
   * - Protocol version
     - ``u32`` little-endian
   * - Minified rules length
     - ``u64`` little-endian
   * - Minified rules
     - UTF-8 bytes
   * - Map name length
     - ``u64`` little-endian
   * - Map name
     - UTF-8 bytes
   * - Player count
     - ``u8``

FNV-1a starts at ``0xcbf29ce484222325`` and multiplies by ``0x100000001b3``
after XOR-ing each byte. Because the checksum covers minified rules, comments
and formatting never affect setup acknowledgment, while any change to a
literal does. ``MatchSetup::checksum`` (rules, map, seed, and players) still
exists for local setups and replay previews; it never crosses the network.
