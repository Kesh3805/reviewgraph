function Deco(): any { return () => {}; }

export class Odd {
  @Deco() @Deco()
  both(): void {}

  @Deco()
  // a comment between decorator and member
  commented(): void {}
}
