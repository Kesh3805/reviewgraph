const fs = require("fs");

function read(path) {
  return fs.readFileSync(path, "utf8");
}

module.exports = { read };
