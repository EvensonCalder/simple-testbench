# Example Scenario

This example benchmark asks each model to extract a todo item, time, and location from natural language and return JSON in this shape:

```json
{
  "todo": "buy milk",
  "time": "2026-04-24T22:13:20Z",
  "location": "supermarket"
}
```

The directory includes:

- one provider definition for OpenRouter
- four model definitions spanning all target API styles
- one strict extraction system prompt
- ten test cases
- one optional Lua post-processor
- one Lua scoring script for JSON validity
- one AI scoring definition for extraction quality

The files are runnable inputs for the implemented STB benchmark pipeline.
