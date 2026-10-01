# frozen_string_literal: true

# Hand-written RFC 8785 (JSON Canonicalization Scheme) encoder.
#
# Derived directly from RFC 8785 sections 3.2 (object member ordering) and
# 3.2.2 (serialization of primitive values), NOT from any JCS/canonical-JSON
# gem. This project intentionally avoids any such library per the task's
# independence constraint.
#
# Scope: this encoder supports exactly the value shapes our own expansion
# produces: Hash, Array, String, Integer, true, false, nil. RFC 8785's
# floating point ("Number") serialization (ECMA-262 Number::toString) is
# deliberately NOT implemented: our fixture expansion never emits a float
# (every numeric field in the symbolic source and in our expansion is a
# small integer such as a bundle "version"), and RFC 8785's own I-JSON
# profile already excludes numbers outside [-(2^53-1), 2^53-1] from
# interoperable canonicalization, so restricting ourselves to integers is a
# spec-derived choice, not a borrowed implementation detail.
module JCS
  module_function

  # Encode a plain Ruby value tree to its RFC 8785 canonical JSON string.
  def encode(value)
    case value
    when Hash
      encode_object(value)
    when Array
      "[" + value.map { |v| encode(v) }.join(",") + "]"
    when String
      encode_string(value)
    when Integer
      encode_integer(value)
    when true
      "true"
    when false
      "false"
    when nil
      "null"
    when Float
      raise ArgumentError, "JCS: floating point values are out of scope for this encoder (RFC 8785 Number serialization not implemented); got #{value.inspect}"
    else
      raise ArgumentError, "JCS: unsupported value type #{value.class}"
    end
  end

  # RFC 8785 3.2.3: object members sorted by UTF-16 code unit sequence of
  # the member name (not codepoint order, not byte order) -- these differ
  # only for names containing characters above U+FFFF or in the
  # noncharacter/surrogate ranges, but we implement the exact rule.
  def encode_object(hash)
    entries = hash.map { |k, v| [k.to_s, v] }
    entries.sort_by! { |k, _| utf16_units(k) }
    "{" + entries.map { |k, v| "#{encode_string(k)}:#{encode(v)}" }.join(",") + "}"
  end

  def utf16_units(str)
    units = []
    str.each_codepoint do |cp|
      if cp <= 0xFFFF
        units << cp
      else
        c = cp - 0x10000
        units << (0xD800 + (c >> 10))
        units << (0xDC00 + (c & 0x3FF))
      end
    end
    units
  end

  # RFC 8785 3.2.2.2 / RFC 8259 string grammar: minimal escaping. Required
  # escapes are quote, backslash, and control characters < 0x20 (with the
  # short forms \b \t \n \f \r where defined); everything else -- including
  # all non-ASCII -- is emitted as raw UTF-8, never as a \uXXXX escape.
  def encode_string(str)
    out = +"\""
    str.each_codepoint do |cp|
      case cp
      when 0x22 then out << "\\\""
      when 0x5C then out << "\\\\"
      when 0x08 then out << "\\b"
      when 0x09 then out << "\\t"
      when 0x0A then out << "\\n"
      when 0x0C then out << "\\f"
      when 0x0D then out << "\\r"
      else
        if cp < 0x20
          out << format("\\u%04x", cp)
        else
          out << [cp].pack("U")
        end
      end
    end
    out << "\""
    out
  end

  def encode_integer(int)
    # I-JSON / RFC 8785-compatible interoperable integer range.
    max = (2**53) - 1
    min = -max
    if int > max || int < min
      raise ArgumentError, "JCS: integer #{int} outside the interoperable range [#{min}, #{max}]"
    end
    int.to_s
  end
end
