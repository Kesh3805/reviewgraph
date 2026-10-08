import { AuthService } from './auth.service';
import { PermissionService } from './permission.service';

describe('AuthService', () => {
  it('denies without permission', async () => {
    const permissions = {
      check: jest.fn().mockResolvedValue(false),
    } as unknown as PermissionService;
    const service = new AuthService(permissions);
    await expect(service.authorize({ id: 'u1', role: 'member' }, { id: 'r1' })).resolves.toBe(false);
  });
});
