# Resources

All files in this directory are unmodified copies of those available on the IEEE 2030.5 downloads page, with the exception of `sep-ordered-deps.xsd`

- `sep-ordered-deps.xsd` is a modified `sep.xsd` that removes all type forward references from `xs:extensions`, as they are currently incompatible with our [xsd parser](https://github.com/lumeohq/xsd-parser-rs).