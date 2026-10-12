export class UserEntity {
  id = '';
  email = '';

  getId() {
    return this.id;
  }

  describe(): string {
    return `${this.id} <${this.email}>`;
  }
}
