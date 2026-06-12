interface P { name: string; age: number }
type Greeting = string;
enum Color { Red, Green, Blue }

function identity<T>(x: T): T { return x; }

const greet = (p: P): Greeting => `hi ${p.name}, ${Color[Color.Green]}`;

export const msg: Greeting = identity(greet({ name: 'ts', age: 1 }));
