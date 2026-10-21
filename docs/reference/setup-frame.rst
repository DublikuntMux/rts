Binary setup frame
==================

The server sends ``ServerMessage::Setup`` as one binary WebSocket message: a
fixed 33-byte header followed by two zlib streams, the rules and then the map.
The format is implemented in ``crates/net/src/setup/wire.rs``. All multibyte
integers are little-endian. There is no base64 or RON wrapping.

Layout
------

.. kroki::
   :type: rackdiag
   :caption: Setup frame sections in byte order, not to scale. R and M are the compressed rules and map lengths.
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

           1: "Header\nOffset 0; 33 bytes";
           2: "Rules zlib stream\nOffset 33; R bytes";
           3: "Map zlib stream\nOffset 33 + R; M bytes";
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
     - Protocol version, currently 4.
   * - 8
     - 8
     - ``u64`` seed
     - Match seed.
   * - 16
     - 1
     - ``u8`` players
     - Player count, 1 to 8.
   * - 17
     - 4
     - ``u32`` rules length
     - Decompressed length of the minified rules.
   * - 21
     - 4
     - ``u32`` R
     - Compressed rules length.
   * - 25
     - 4
     - ``u32`` map length
     - Decompressed length of the minified map.
   * - 29
     - 4
     - ``u32`` M
     - Compressed map length.

The frame's total length must be exactly ``33 + R + M``.

Encoding
--------

For each document the server:

1. Checks that the original text is at most 4 MiB.
2. Minifies it lexically: removes comments and whitespace that does not
   separate tokens. String, character, raw-string, and byte literals, numbers,
   identifiers, and attributes keep their exact spelling.
3. Compresses the minified text as one zlib stream with DEFLATE level 6, using
   the pure-Rust ``miniz_oxide`` codec, and checks that the stream is at most
   8 MiB.

The minifier never parses and reserializes numbers. Decimal positions and
speeds are converted to world units by every client from the same spelling,
so the result stays bit-identical. Minifying is idempotent; malformed input is
copied through and rejected later by validation.

Decoding
--------

A client accepts the frame only if every check passes, before it sends
``Ready``:

1. The frame is at most ``33 + 2 × 8 MiB`` bytes and at least 33 bytes.
2. The tag is ``RTSS`` and the version matches.
3. The player count is 1 to 8.
4. Each declared decompressed length is at most 4 MiB, and each compressed
   length is at most 8 MiB.
5. The payload splits exactly into R and M bytes.
6. Each stream inflates into a buffer of exactly the declared length,
   ends with a valid zlib checksum, and consumes all of its input. Truncated,
   trailing, corrupt, or expanding data is rejected.
7. Each document is valid UTF-8.
8. The rules and map parse, the map's player count equals the header's, and
   the full initial simulation builds.

Allocation is bounded by the declared length, which is checked before the
buffer is created, so a small frame cannot claim unbounded memory.

Checksum
--------

``MatchSetup::checksum`` is the 64-bit FNV-1a hash of, in order:

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
   * - Minified map length
     - ``u64`` little-endian
   * - Minified map
     - UTF-8 bytes
   * - Seed
     - ``u64`` little-endian
   * - Player count
     - ``u8``

FNV-1a starts at ``0xcbf29ce484222325`` and multiplies by ``0x100000001b3``
after XOR-ing each byte. Because the checksum covers minified text, comments
and formatting never affect readiness, while any change to a literal does.
