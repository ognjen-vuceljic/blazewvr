%dw 2.0
output application/json
---
{
    orderCount: sizeOf(payload.orders)
}
