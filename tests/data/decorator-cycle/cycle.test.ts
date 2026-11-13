import assert from 'node:assert/strict';
import { fieldTypes } from './metadata';
import { Role } from './role';
import { PersonRole } from './person-role';
import { PartnerProfile } from './partner-profile';
import { Customer } from './customer';

test('cyclic Role metadata observes a partial CommonJS export instead of a TDZ getter', () => {
  assert.equal(typeof Role, 'function');
  assert.equal(typeof PersonRole, 'function');
  assert.equal(fieldTypes.get(PersonRole.prototype)?.get('role'), Object);
  assert.equal(fieldTypes.get(Role.prototype)?.get('assignments'), Array);
});
test('cyclic PartnerProfile metadata finishes initializing both decorated classes', () => {
  assert.equal(typeof PartnerProfile, 'function');
  assert.equal(typeof Customer, 'function');
  assert.equal(fieldTypes.get(Customer.prototype)?.get('partnerProfile'), Object);
  assert.equal(fieldTypes.get(PartnerProfile.prototype)?.get('customer'), Customer);
});
test('initialized circular exports retain constructor identity', () => {
  assert.equal(new Role() instanceof Role, true);
  assert.equal(new PartnerProfile() instanceof PartnerProfile, true);
});
