export interface Shape {
  readonly name: string;
  area?(): number;
  scale(factor: number): Shape;
  scale(x: number, y: number): Shape;
}

export abstract class Base implements Shape {
  abstract readonly name: string;
  abstract area(): number;
  scale(factor: number): Shape {
    return this;
  }
}

export class Circle extends Base {
  static readonly PI = 3.14159;
  readonly name = "circle";
  #radius: number;
  constructor(private readonly r: number, public label?: string) {
    super();
    this.#radius = r;
  }
  get diameter(): number {
    return this.r * 2;
  }
  set diameter(value: number) {
    this.#radius = value / 2;
  }
  area(): number {
    return Circle.PI * this.#radius ** 2;
  }
  static unit(): Circle {
    return new Circle(1);
  }
}

export type Point = { x: number; y: number };
export type Maybe<T> = T | undefined;
