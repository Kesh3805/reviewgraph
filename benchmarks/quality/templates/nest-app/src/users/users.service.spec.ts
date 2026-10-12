import { UsersRepository } from './users.repository';
import { UsersService } from './users.service';

describe('UsersService', () => {
  it('formats the display name', async () => {
    const repo = { findUser: async () => ({ firstName: 'Ada', lastName: 'Lovelace' }) };
    const service = new UsersService(repo as unknown as UsersRepository);
    expect(await service.displayName('u1')).toBe('Ada Lovelace');
  });
});
