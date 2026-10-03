import { readFile } from "node:fs/promises";

export async function load(path) {
  return readFile(path, "utf8");
}
