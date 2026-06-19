// Decorated model in a require cycle with Post (User -> Post -> User), mirroring
// Sequelize/Mongoose models that reference each other. The decorator runs at class-eval time.
import { Entity, Field } from './decorators';
import { Post } from './post.model';

@Entity('users')
export class User {
  @Field({ type: 'string' })
  declare name: string;

  // lazy back-reference into the cyclic dep — resolved when called, not at module eval.
  posts(): string {
    return 'posts-of:' + Post.entityName;
  }
}
