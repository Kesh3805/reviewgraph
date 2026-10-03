import { Injectable } from "@nestjs/common";

@Injectable()
export class UserService {
  private cache = new Map<string, string>();
  handler = async (id: string) => this.cache.get(id);

  constructor(private readonly repo: UserRepository) {}

  @Log()
  async find(id: string): Promise<string | undefined> {
    return this.repo.find(id);
  }
}

declare class UserRepository {
  find(id: string): Promise<string | undefined>;
}

declare function Log(): MethodDecorator;
