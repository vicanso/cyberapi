; GraphQL, onto the captures gpui-component's theme colours (see its
; json / typescript highlights.scm). The grammar is tree-sitter-graphql's.

(comment) @comment
(description) @comment

(string_value) @string
(int_value) @number
(float_value) @number
(boolean_value) @boolean
(null_value) @constant.builtin
(enum_value) @constant

(variable) @variable.parameter

(operation_type) @keyword
[
  "query"
  "mutation"
  "subscription"
  "fragment"
  "on"
  "type"
  "interface"
  "union"
  "enum"
  "input"
  "scalar"
  "schema"
  "extend"
  "directive"
  "implements"
  "repeatable"
] @keyword

(operation_definition (name) @function)
(fragment_name (name) @function)
(directive "@" @function.builtin)
(directive (name) @function.builtin)

(field (name) @property)
(alias (name) @variable)
(argument (name) @variable.parameter)
(object_field (name) @property)
(field_definition (name) @property)
(input_value_definition (name) @variable.parameter)

(named_type (name) @type)
(object_type_definition (name) @type)
(interface_type_definition (name) @type)
(input_object_type_definition (name) @type)
(enum_type_definition (name) @type)
(union_type_definition (name) @type)
(scalar_type_definition (name) @type)

[
  "{"
  "}"
  "("
  ")"
  "["
  "]"
] @punctuation.bracket

[
  ":"
  "="
  "|"
  "&"
  "!"
  "..."
] @punctuation.delimiter
