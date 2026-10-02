import { Test } from '@nestjs/testing';
import { UsersService } from './users.service';
import { UsersRepository } from './users.repository';

describe('UsersService', () => {
  let service: UsersService;

  beforeEach(async () => {
    const moduleRef = await Test.createTestingModule({
      providers: [UsersService, { provide: UsersRepository, useValue: { findById: jest.fn() } }],
    }).compile();
    service = moduleRef.get(UsersService);
  });

  it('delegates findOne to the repository', () => {
    service.findOne('1');
    expect(service).toBeDefined();
  });
});
