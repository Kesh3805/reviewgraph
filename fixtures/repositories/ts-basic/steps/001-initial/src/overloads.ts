export class Parser {
  parse(input: string): string;
  parse(input: number): number;
  parse(input: any): any {
    return input;
  }
}
