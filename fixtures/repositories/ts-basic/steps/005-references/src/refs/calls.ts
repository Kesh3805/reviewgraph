import { shout } from '../util/strings';

export class Base {
  protected init(): void {}
}

export class Derived extends Base {
  constructor() {
    super();
  }

  run(): string {
    this.init();
    super.init();
    return shout('a');
  }
}

setup();

function setup(): void {}
