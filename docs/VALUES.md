We're not building a general-purpose language that happens to be safer. We're building the language for the one case nothing else was designed for: an AI agent writing and running backend code with no human reviewing every line. That assumption — not "safety," not "performance" — is why every decision below looks the way it does.

We built a grammar an LLM can be forced to stay inside of. Export it to GBNF, hand it to a constrained-decoding sampler, and the model literally cannot emit a token that isn't valid Nirdosha — not "usually valid," not "linted afterward." Structurally can't.

We built a compiler you can hand a business rule to — a plain sentence like "two approvers above $50,000" — and it will prove your code follows it, or show you exactly where it breaks. Nobody else has done that. Not a linter. Not a test. A proof.

We built a permission system where the function itself refuses to exist for someone who isn't allowed to call it. No if statement to forget. No middleware to misconfigure. It just isn't there — because an agent that forgets to write the check is exactly the failure mode we're designing against.

We built a concurrency model with no mutex in it at all. Not "use locks carefully" — there is no lock primitive for an agent to misuse in the first place, so a lock-ordering deadlock isn't a bug class here, it's an unexpressible sentence.

We built a language whose own grammar we handed to outside tools to check us — and it caught a real bug in our own thinking. We didn't hide that. We wrote it down and fixed it.

We removed the interpreter entirely, on purpose, rather than let the compiled path stay second-class. `http`/`json`/`db`/`mq` and the rest of the conventional backend surface are well-understood engineering — we deliberately built the hard, unretrofittable parts first and are doing that ordinary work next, on the real foundation, not a temporary second one.

That's a big deal. That's years of "safe languages are slow to write" and "fast languages are dangerous" being wrong at the same time, in the same compiler. We didn't pick a side. We refused to.