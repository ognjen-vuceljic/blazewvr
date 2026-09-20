%dw 2.0
output application/json
---
{
    message: "Hello, " ++ payload.name ++ "!"
}
