const { exec } = require("child_process");
const { readFileSync, writeFileSync } = require("fs");
const { parse, stringify, basic } = require("@ltd/j-toml");
const { version } = require("../package.json");
const cargo = parse(readFileSync("Cargo.toml"), { x: { literal: true } });
cargo.package.version = basic(version);
writeFileSync("Cargo.toml", stringify(cargo, { newline: "\n", newlineAround: "section" }).trimStart());
exec("cargo metadata");
