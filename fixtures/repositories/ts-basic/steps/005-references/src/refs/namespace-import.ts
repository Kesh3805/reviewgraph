import * as fs from 'fs';
import defaultApi from './api';

export function read(p: string) {
  fs.readFileSync(p);
  return defaultApi.get(p);
}
