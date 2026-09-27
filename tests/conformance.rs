//! The host tables, checked against what `NeetComputers` implements.

mod common;

use common::{expect_clean, TestDisk};

#[test]
fn files_open_rejects_what_upstream_rejects() {
    let disk = TestDisk::new("files-open");
    // `FilesAPI.open`.
    expect_clean(disk.run(
        r#"
        raises("directory",      "Not a file",         files.open, "system:/sub", "r")
        raises("missing",        "Not a file",         files.open, "system:/nope.txt", "r")
        raises("bad mode",       "Invalid open mode",  files.open, "system:/hello.txt", "zz")
        raises("readonly write", "Access denied",      files.open, "rom:/readonly.txt", "w")
        raises("bad disk",       "Disk not found",     files.open, "system:/hello.txt", "r", 9)

        local h = files.open("system:/hello.txt", "r")
        check("handle is a table", type(h) == "table", type(h))
        for _, name in ipairs({ "read", "write", "seek", "flush", "close" }) do
            check("handle." .. name, type(h[name]) == "function")
        end
        h.close()

        -- A readonly partition still opens for reading.
        local r = files.open("rom:/readonly.txt", "r")
        same("readonly read", r.read("a"), "rom")
        r.close()
    "#,
    ));
}

#[test]
fn file_modes_match_filehelper() {
    let disk = TestDisk::new("files-modes");
    // `FileHelper.getMode`, with the three-character modes enabled.
    expect_clean(disk.run(
        r#"
        -- Each mode gets its own file.
        for i, mode in ipairs({ "r", "w", "a", "r+", "w+", "a+", "rb", "wb", "ab",
                                "rb+", "r+b", "wb+", "w+b", "ab+", "a+b" }) do
            local path = "system:/mode" .. i .. ".txt"
            local seed = files.open(path, "w")
            seed.write("seed")
            seed.close()

            local ok, handle = pcall(files.open, path, mode)
            check("mode " .. mode .. " accepted", ok, handle)
            if ok then handle.close() end
        end
        for _, mode in ipairs({ "", "zz", "rw", "q", "rbb" }) do
            raises("mode " .. mode .. " rejected", "Invalid open mode",
                files.open, "system:/hello.txt", mode)
        end
        -- An absent mode is "r".
        local h = files.open("system:/hello.txt")
        same("default mode reads", h.read("a"), "hello\nworld\n")
        h.close()
    "#,
    ));
}

#[test]
fn handle_reads_match_fileheader() {
    let disk = TestDisk::new("files-read");
    // `FileHeader.read`, with nil at end of file on a numeric read.
    expect_clean(disk.run(
        r#"
        local h = files.open("system:/hello.txt", "r")
        same("read l",        h.read("l"), "hello")
        same("read L",        h.read("L"), "world\n")
        same("read at eof",   h.read("l"), nil)
        h.close()

        h = files.open("system:/hello.txt", "r")
        same("read all", h.read("a"), "hello\nworld\n")
        same("read a at eof", h.read("a"), nil)
        h.close()

        h = files.open("system:/hello.txt", "r")
        same("read 5",   h.read(5), "hello")
        same("cursor",   h.seek(), 5)
        same("read back", h.read(-5), "hello")
        same("rewound",  h.seek(), 0)
        h.close()

        h = files.open("system:/noeol.txt", "r")
        same("unterminated line", h.read("l"), "tail")
        h.close()

        h = files.open("system:/empty.txt", "r")
        same("empty read a", h.read("a"), nil)
        h.close()

        h = files.open("system:/hello.txt", "r")
        same("read 0", h.read(0), "")
        raises("bad format", "Invalid format", h.read, "q")
        raises("long format", "Invalid format", h.read, "la")
        h.close()
        raises("read when closed", "Attempt to use a closed file", h.read, "a")
        raises("close twice", "Attempt to use a closed file", h.close)

        -- Access denied, not a silent empty read.
        local w = files.open("system:/scratch.txt", "w")
        raises("read a write handle", "Access denied", w.read, "a")
        w.close()
    "#,
    ));
}

#[test]
fn binary_bytes_survive_a_text_round_trip() {
    let disk = TestDisk::new("files-binary");
    // Bytes that are not UTF-8 survive text mode.
    expect_clean(disk.run(
        r#"
        local h = files.open("system:/binary.dat", "rb")
        local data = h.read("a")
        h.close()
        same("binary length", #data, 5)
        same("binary byte 1", data:byte(1), 0xFF)
        same("binary byte 2", data:byte(2), 0xFE)
        same("binary byte 3", data:byte(3), 0x00)

        local t = files.open("system:/binary.dat", "r")
        local text = t.read("a")
        t.close()
        same("text mode is byte exact", #text, 5)
        same("text byte 1", text:byte(1), 0xFF)
    "#,
    ));
    assert_eq!(
        disk.read("system/binary.dat"),
        vec![0xFF, 0xFE, 0x00, 0x01, b'z']
    );
}

#[test]
fn handle_seek_matches_fileheader() {
    let disk = TestDisk::new("files-seek");
    expect_clean(disk.run(
        r#"
        local h = files.open("system:/hello.txt", "r")
        same("seek with no args", h.seek(), 0)
        same("seek set",   h.seek("set", 6), 6)
        same("seek cur",   h.seek("cur", 2), 8)
        same("seek end",   h.seek("end"), 12)
        same("seek end -2", h.seek("end", -2), 10)
        raises("seek past end",   "Invalid offset", h.seek, "set", 99)
        raises("seek negative",   "Invalid offset", h.seek, "set", -1)
        raises("seek end ahead",  "Invalid offset", h.seek, "end", 1)
        raises("seek cur under",  "Invalid offset", h.seek, "cur", -99)
        raises("bad whence",      "Invalid option", h.seek, "middle")
        h.close()
    "#,
    ));
}

#[test]
fn writes_land_at_the_cursor() {
    let disk = TestDisk::new("files-write");
    // Writes at the cursor.
    expect_clean(disk.run(
        r#"
        local h = files.open("system:/out.txt", "w")
        h.write("hello")
        h.seek("set", 0)
        h.write("J")
        h.close()

        local r = files.open("system:/out.txt", "r")
        same("write at cursor", r.read("a"), "Jello")
        r.close()

        -- Append opens at the end.
        local a = files.open("system:/out.txt", "a")
        a.write("!")
        a.close()
        r = files.open("system:/out.txt", "r")
        same("append", r.read("a"), "Jello!")
        r.close()

        -- A number writes one byte.
        local b = files.open("system:/byte.bin", "wb")
        b.write(65)
        b.close()
        r = files.open("system:/byte.bin", "rb")
        same("numeric write", r.read("a"), "A")
        r.close()

        local ro = files.open("system:/hello.txt", "r")
        raises("write a read handle", "Access denied", ro.write, "x")
        ro.close()
    "#,
    ));
    assert_eq!(disk.read("system/out.txt"), b"Jello!");
}

#[test]
fn paths_fold_case_and_stay_inside_the_partition() {
    let disk = TestDisk::new("files-paths");
    // Path lookup ignores case.
    expect_clean(disk.run(
        r#"
        local h = files.open("system:/mixedcase.txt", "r")
        same("lowercased path", h.read("a"), "mixed")
        h.close()
        h = files.open("SYSTEM:/MIXEDCASE.TXT", "r")
        same("uppercased path", h.read("a"), "mixed")
        h.close()

        check("exists",  files.exists("system:/hello.txt"))
        check("isFile",  files.isFile("system:/hello.txt"))
        check("isDir",   files.isDir("system:/sub"))
        check("not a dir", not files.isDir("system:/hello.txt"))
        check("missing", not files.exists("system:/nope"))

        -- The escape forms are rejected by the path grammar, not resolved.
        for _, bad in ipairs({ "system:/../../etc/passwd", "system:/..", "system:/." }) do
            check("rejected " .. bad, not files.exists(bad))
        end
        check("unknown partition", not files.exists("nosuch:/x"))

        local kids = files.getChildren("system:/sub")
        same("children count", #kids, 2)
        same("children sorted 1", kids[1], "a.txt")
        same("children sorted 2", kids[2], "b.txt")
        raises("children of a file", "Not a directory", files.getChildren, "system:/hello.txt")
        raises("children of nothing", "File does not exist", files.getChildren, "system:/nope")
    "#,
    ));
}

#[test]
fn disk_and_partition_queries_match_filesapi() {
    let disk = TestDisk::new("files-disks");
    // `DiskManager` addresses disks by slot, so the boot disk is 0.
    expect_clean(disk.run(
        r#"
        local disks = files.getDisks()
        same("one disk", #disks, 1)
        same("addressed as 0", disks[1], 0)
        same("count", files.getNumberOfDisks(), 1)
        check("uuid is a string", type(files.getDiskID(0)) == "string", files.getDiskID(0))
        raises("bad disk id", "Disk not found", files.getDiskID, 7)

        local parts = files.getPartitions()
        same("two partitions", #parts, 2)
        same("first partition", parts[1], "system")

        local p = files.getPartition("rom")
        same("partition name", p.name, "rom")
        same("partition readonly", p.readonly, true)
        same("partition hidden", p.hidden, false)
        same("unknown partition", files.getPartition("nope"), nil)
    "#,
    ));
}

#[test]
fn event_queues_match_eventmanager() {
    let disk = TestDisk::new("events");
    // `EventManager`: six labels, cap 75, oldest dropped, `getQueue` drains.
    expect_clean(disk.run(
        r#"
        for _, name in ipairs({ "Unlabeled", "User", "System", "Network",
                                "Peripheral", "Compatibility" }) do
            local ok, err = pcall(event.getQueue, name)
            check("label " .. name, ok, err)
        end
        check("case insensitive", pcall(event.getQueue, "USER"))
        raises("bad label", "Invalid event category 'Nope'", event.getQueue, "Nope")

        event.queueEvent("User", "keyPressed", 65, "A", 1)
        local q = event.getQueue("User")
        same("one event", #q, 1)
        same("name first", q[1][1], "keyPressed")
        same("arg 1", q[1][2], 65)
        same("arg 2", q[1][3], "A")
        same("arg 3", q[1][4], 1)
        same("drained", #event.getQueue("User"), 0)

        -- A fresh array each time.
        event.queueEvent("User", "a")
        local first = event.getQueue("User")
        table.remove(first, 1)
        event.queueEvent("User", "b")
        same("fresh array", #event.getQueue("User"), 1)

        -- Overflow drops the oldest, capped at 75.
        for i = 1, 80 do event.queueEvent("System", "e" .. i) end
        local overflowed = event.getQueue("System")
        same("capped at 75", #overflowed, 75)
        same("oldest dropped", overflowed[1][1], "e6")
        same("newest kept", overflowed[75][1], "e80")

        -- Queues are independent.
        event.queueEvent("User", "u")
        event.queueEvent("Network", "n")
        same("user queue", #event.getQueue("User"), 1)
        same("network queue", #event.getQueue("Network"), 1)

        -- A filtered take removes every match, including the first.
        event.queueEvent("User", "keep")
        event.queueEvent("User", "drop")
        event.queueEvent("User", "drop")
        same("filtered count", #event.getQueue("User", "drop"), 2)
        local rest = event.getQueue("User")
        same("only the keeper left", #rest, 1)
        same("keeper", rest[1][1], "keep")

        -- getFirst pops one.
        event.queueEvent("User", "x")
        event.queueEvent("User", "y")
        same("getFirst", event.getFirst("User")[1], "x")
        same("one left", #event.getQueue("User"), 1)
        same("getFirst on empty", event.getFirst("User"), nil)

        event.queueEvent("User", "z")
        event.clear("User")
        same("cleared", #event.getQueue("User"), 0)
    "#,
    ));
}

#[test]
fn bit32_matches_bit32compat() {
    let disk = TestDisk::new("bit32");
    // `Bit32Compat`, with exact results above 2^24.
    expect_clean(disk.run(r#"
        same("band",    bit32.band(0xF0F0, 0xFF00), 0xF000)
        same("bor",     bit32.bor(0xF0, 0x0F), 0xFF)
        same("bxor",    bit32.bxor(0xFF, 0x0F), 0xF0)
        same("bnot",    bit32.bnot(0), 4294967295)
        same("bnot 1",  bit32.bnot(1), 4294967294)
        check("btest true",  bit32.btest(0xF0, 0x10))
        check("btest false", not bit32.btest(0xF0, 0x01))

        same("lshift",       bit32.lshift(1, 4), 16)
        same("lshift wide",  bit32.lshift(1, 31), 2147483648)
        same("lshift over",  bit32.lshift(1, 32), 0)
        same("lshift neg",   bit32.lshift(16, -4), 1)
        same("rshift",       bit32.rshift(16, 4), 1)
        same("rshift over",  bit32.rshift(1, 32), 0)
        same("rshift high",  bit32.rshift(0x80000000, 31), 1)
        same("arshift",      bit32.arshift(0x80000000, 31), 4294967295)
        same("arshift pos",  bit32.arshift(16, 4), 1)

        same("lrotate",      bit32.lrotate(0x80000000, 1), 1)
        same("lrotate zero", bit32.lrotate(0x12345678, 0), 0x12345678)
        same("rrotate",      bit32.rrotate(1, 1), 2147483648)
        same("rotate wrap",  bit32.lrotate(0x12345678, 32), 0x12345678)

        same("extract",       bit32.extract(0xF0, 4, 4), 0xF)
        same("extract 1 bit", bit32.extract(0x2, 1), 1)
        same("extract full",  bit32.extract(0xFFFFFFFF, 0, 32), 4294967295)
        same("replace",       bit32.replace(0x00, 0xF, 4, 4), 0xF0)

        raises("extract negative field", "bad argument #2 to 'extract' (field cannot be negative)",
            bit32.extract, 1, -1)
        raises("extract overrun", "trying to access non-existent bits", bit32.extract, 1, 30, 4)
        raises("band no args", "bad argument #1 to 'band' (number expected, got no value)", bit32.band)

        -- Every result stays an exact integer.
        for _, v in ipairs({ 0x80000000, 0xFFFFFFFF, 0xFFFFFF01, 0x80000001 }) do
            local got = bit32.bor(v, 0)
            same("exact " .. string.format("%x", v), got, v)
            same("integral " .. string.format("%x", v), math.type(got), "integer")
        end
    "#));
}

#[test]
fn screen_matches_graphicalapi() {
    let disk = TestDisk::new("screen");
    // `GraphicalAPI` at 0.5.3: four-byte readData, overhang rejection, inclusive corners.
    expect_clean(disk.run(
        r#"
        local w, h = screen.getSize()
        same("width", w, 800)
        same("height", h, 600)

        for _, name in ipairs({ "getSize", "substitute", "clone", "writeData", "readData",
                                "writePixel", "writeLine", "readPixel", "set", "fill",
                                "draw", "createLayer" }) do
            check("screen." .. name, type(screen[name]) == "function", type(screen[name]))
        end

        -- readData is four bytes per pixel with alpha forced opaque.
        screen.set()
        screen.writePixel(0, 0, 10, 20, 30, 255)
        local data = screen.readData(0, 0, 0, 0)
        same("one pixel is 4 bytes", #data, 4)
        same("red",   data:byte(1), 10)
        same("green", data:byte(2), 20)
        same("blue",  data:byte(3), 30)
        same("alpha forced", data:byte(4), 255)

        -- Corners are inclusive at both ends.
        same("rect stride", #screen.readData(0, 0, 3, 1), 4 * 2 * 4)

        local r, g, b = screen.readPixel(0, 0)
        same("readPixel red",   r, 10)
        same("readPixel green", g, 20)
        same("readPixel blue",  b, 30)

        -- writePixel clips silently; the rect calls raise.
        screen.writePixel(-1, -1, 1, 2, 3, 255)
        screen.writePixel(10000, 0, 1, 2, 3, 255)
        raises("clone x order", "x2 must be larger then x1", screen.clone, 5, 0, 1, 0)
        raises("clone y order", "y2 must be larger then y1", screen.clone, 0, 5, 0, 1)
        raises("readData off screen", nil, screen.readData, 0, 0, 800, 0)
        raises("fill off screen",     nil, screen.fill, 0, 0, 0, 600, 1, 2, 3)
        raises("readPixel off screen", nil, screen.readPixel, 800, 0)

        -- writeData rejects an overhanging buffer instead of cropping.
        local row = string.rep(string.char(1, 2, 3, 255), 4)
        local ok = pcall(screen.writeData, 798, 0, row, 4)
        check("overhang rejected", not ok)
        raises("ragged buffer", "Length of buffer must by dividable by 4",
            screen.writeData, 0, 0, "abc", 1)
        raises("width mismatch", "Length of buffer must by dividable by width",
            screen.writeData, 0, 0, row, 3)

        screen.writeData(0, 0, row, 4)
        local back = screen.readData(0, 0, 3, 0)
        same("writeData round trip", back, row)

        -- Height is inferred from the buffer length.
        local block = string.rep(string.char(9, 9, 9, 255), 8)
        screen.writeData(10, 10, block, 4)
        same("inferred height", #screen.readData(10, 10, 13, 11), 8 * 4)

        -- An alpha of 0 leaves the destination alone; 255 replaces it.
        screen.fill(0, 0, 1, 0, 100, 100, 100, 255)
        screen.writeData(0, 0, string.char(7, 7, 7, 0) .. string.char(8, 8, 8, 255), 2)
        local blended = screen.readData(0, 0, 1, 0)
        same("alpha 0 keeps", blended:byte(1), 100)
        same("alpha 255 replaces", blended:byte(5), 8)

        -- set() clears, set(r,g,b) fills.
        screen.set()
        same("cleared", screen.readData(0, 0, 0, 0):byte(1), 0)
        screen.set(1, 2, 3)
        local filled = screen.readData(500, 400, 500, 400)
        same("filled red", filled:byte(1), 1)
        same("filled blue", filled:byte(3), 3)

        -- substitute ignores alpha and swaps packed colours.
        screen.substitute(1, 2, 3, 4, 5, 6)
        same("substituted", screen.readData(0, 0, 0, 0):byte(1), 4)

        -- Layers carry the same surface API, and clone returns one.
        local layer = screen.createLayer(4, 4)
        local lw, lh = layer.getSize()
        same("layer width", lw, 4)
        same("layer height", lh, 4)
        check("layer nests", type(layer.createLayer) == "function")
        raises("zero size layer", "Size cant be zero or less", screen.createLayer, 0, 4)

        layer.set(9, 9, 9)
        local copy = layer.clone(0, 0, 1, 1)
        local cw, ch = copy.getSize()
        same("clone width", cw, 2)
        same("clone height", ch, 2)
        same("clone content", copy.readData(0, 0, 0, 0):byte(1), 9)

        -- The screen is usable as a layer.
        same("screen as layer", #screen.readData(0, 0, 0, 0), 4)
    "#,
    ));
}

#[test]
fn chip_matches_chipapi() {
    let disk = TestDisk::new("chip");
    expect_clean(disk.run(
        r#"
        check("version is a string", type(chip.version()) == "string", chip.version())
        check("machine is a string", type(chip.getMachine()) == "string", chip.getMachine())
        check("uuid is a string", type(chip.getUUID()) == "string")

        local t = chip.getUnixTime()
        check("unix time is a number", type(t) == "number", type(t))
        check("unix time is plausible", t > 1600000000, t)
        -- Finer than a second.
        check("unix time is fractional", math.type(t) == "float", math.type(t))

        check("getTime is a number", type(chip.getTime()) == "number")
        local lunar = chip.getLunarTime()
        check("lunar in range", lunar >= 0 and lunar < 24000, lunar)
    "#,
    ));
}

#[test]
fn io_reports_no_peripherals() {
    let disk = TestDisk::new("io");
    // `IOAPI` with nothing attached.
    expect_clean(disk.run(
        r#"
        local peripherals = io.getPeripherals()
        check("list is a table", type(peripherals) == "table")
        same("nothing attached", #peripherals, 0)
        same("isViewed", io.isViewed(), false)
        raises("bad uuid", "UUID invalidly formatted", io.getType, "not-a-uuid")
        raises("unknown uuid", "Peripheral not found",
            io.getType, "00000000-0000-4000-8000-000000000001")
    "#,
    ));
}

#[test]
fn the_stock_globals_the_mod_removes_are_gone() {
    let disk = TestDisk::new("globals");
    // `LuaBridge.cpp:57-68`.
    expect_clean(disk.run(r#"
        for _, name in ipairs({ "os", "dofile", "loadfile", "collectgarbage", "package", "warn" }) do
            same("no " .. name, _G[name], nil)
        end
        -- `io` is nilled and then replaced by the peripheral table.
        check("io is the peripheral table", type(io) == "table" and io.getPeripherals ~= nil)
        check("_G is writable", pcall(function() _G.marker = 1 end))
        check("require is assignable", pcall(function() _G.require = function() end end))
        check("load survives", type(load) == "function")
        check("debug.sethook survives", type(debug.sethook) == "function")
        check("coroutine.memoryused survives", type(coroutine.memoryused) == "function")
    "#));
}
