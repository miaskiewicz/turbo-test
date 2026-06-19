import { describe, it, expect } from 'vitest';
// Decorated classes (@Entity / @Field) in a require cycle (User <-> Post). Before the
// legacy-decorator-lowering fix, the native transform either emitted `export @Entity class …`
// ("Unexpected token 'export'") or lowered with 2022-standard semantics that re-read the class
// binding before init ("Cannot access 'X' before initialization"). Both must load now.
import { User } from './user.model';
import { Post } from './post.model';

describe('circular decorated models load', () => {
  it('both decorated classes load without a syntax/TDZ error', () => {
    expect(User.entityName).toBe('users');
    expect(Post.entityName).toBe('posts');
  });

  it('decorator ran at class-definition time', () => {
    expect(new User().posts()).toBe('posts-of:posts');
    expect(new Post().author()).toBe('author-of:users');
  });
});
