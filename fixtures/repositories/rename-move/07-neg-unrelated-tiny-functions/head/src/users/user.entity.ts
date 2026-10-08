export class UserEntity {
  id = '';
  email = '';

  describe(): string {
    return `${this.id} <${this.email}>`;
  }
}
