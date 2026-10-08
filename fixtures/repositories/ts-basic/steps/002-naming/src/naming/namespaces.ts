export namespace Outer.Inner.Deepest {
  export const value = 1;
}

declare module "external-package" {
  export function ambient(): void;
}

declare global {
  interface Window {
    appName: string;
  }
}