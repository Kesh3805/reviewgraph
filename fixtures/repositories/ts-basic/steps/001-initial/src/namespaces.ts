export namespace A.B.C {
  export const value = 1;
  export class Inner {}
}

declare module "pkg" {
  export function f(): void;
}

declare global {
  interface Window {
    appName: string;
  }
}
