Binary setup frame
==================

The server sends ``ServerMessage::Setup(LobbySetup)`` as one binary WebSocket
message. The frame is a fixed 25-byte header followed by two zlib streams:
the rules, then the map *name*. The format is implemented in
``crates/net/src/setup/wire.rs``. All multibyte integers are little-endian.
There is no base64 or RON wrapping.

The setup is public. It never contains the map's terrain, units, deposits,
starts, or the match seed. Each player learns the map only from its own
view frames; see :doc:`../architecture/visibility`.

Layout
------

.. kroki::
   :type: rackdiag
   :caption: Setup frame sections in byte order, not to scale. R and M are the compressed rules and map name lengths.
   :align: center

   rackdiag {
       node_width = 420;
       node_height = 72;
       default_node_color = "#f1f5f9";
       linecolor = "#475569";
       textcolor = "#0f172a";

       rack {
           3U;
           ascending = true;

           1: "Header\nOffset 0; 25 bytes";
           2: "Rules zlib stream\nOffset 25; R bytes";
           3: "Map name zlib stream\nOffset 25 + R; M bytes";
       }
   }

.. list-table:: Header fields
   :header-rows: 1
   :widths: 12 12 22 54

   * - Offset
     - Size
     - Field
     - Meaning
   * - 0
     - 4
     - Tag
     - ASCII ``RTSS``.
   * - 4
     - 4
     - ``u32`` version
     - Protocol version, currently 9.
   * - 8
     - 1
     - ``u8`` players
     - Player count, 1 to 8.
   * - 9
     - 4
     - ``u32`` rules length
     - Decompressed length of the minified rules.
   * - 13
     - 4
     - ``u32`` R
     - Compressed rules length.
   * - 17
     - 4
     - ``u32`` name length
     - Decompressed length of the map name in bytes.
   * - 21
     - 4
     - ``u32`` M
     - Compressed name length.

The frame's total length must be exactly ``25 + R + M``.

Encoding
--------

The server encodes the rules as follows:

1. Checks that the original text is at most 4 MiB.
2. Minifies it lexically: removes comments and whitespace that does not
   separate tokens. String, character, raw-string, and byte literals, numbers,
   identifiers, and attributes keep their exact spelling.
3. Compresses the minified text as one zlib stream with DEFLATE level 6, using
   the pure-Rust ``miniz_oxide`` codec, and checks that the stream is at most
   8 MiB.

The map name is compressed as-is, without minifying.

The minifier never parses and reserializes numbers. Decimal speeds and
distances are converted to world units by every peer from the same spelling.
Minifying is idempotent; malformed input is copied through and rejected later
by validation.

Decoding
--------

A client accepts the frame only if every check passes, before it sends
``Loaded { checksum }``:

1. The frame is at most ``25 + 2 × 8 MiB`` bytes and at least 25 bytes.
2. The tag is ``RTSS`` and the version matches.
3. The player count is 1 to 8.
4. The declared rules length is at most 4 MiB and the name length at most
   256 bytes; each compressed length is at most 8 MiB.
5. The payload splits exactly into R and M bytes.
6. Each stream inflates into a buffer of exactly the declared length,
   ends with a valid zlib checksum, and consumes all of its input. Truncated,
   trailing, corrupt, or expanding data is rejected.
7. Each document is valid UTF-8, and the name has at most 64 characters and
   no control characters.
8. The rules parse and validate.

Allocation is bounded by the declared length, which is checked before the
buffer is created, so a small frame cannot claim unbounded memory.

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
