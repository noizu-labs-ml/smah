defmodule McpVtool.Json do
  @moduledoc """
  Minimal JSON codec with **order-preserving objects**.

  The MCP memo key is `sha256(tool name <> 0x00 <> canonical arguments JSON)`
  where the canonical form is what `serde_json` (with `preserve_order`)
  emitted when the Rust lane froze the memo: object keys in the order they
  appeared in the request text, minimal escaping, plain integers. The stdlib
  decoder returns unordered maps, which cannot reproduce that form — so this
  module decodes objects into `{:obj, keyword_list}` and encodes them back
  byte-for-byte for the string/integer shapes virtual-tool arguments carry.

  Deliberately not a general-purpose JSON library: objects, arrays,
  strings, numbers, booleans, null — nothing else. Floats encode via the
  shortest round-trip form, which matches serde for common values but is
  not guaranteed byte-identical for exotics.
  """

  @type json ::
          nil
          | true
          | false
          | integer()
          | float()
          | String.t()
          | [json()]
          | {:obj, [{String.t(), json()}]}

  @spec decode(String.t()) :: {:ok, json()} | {:error, String.t()}
  def decode(binary) when is_binary(binary) do
    case parse_value(ws(binary)) do
      {:ok, value, rest} ->
        case ws(rest) do
          <<>> -> {:ok, value}
          _ -> {:error, "trailing characters after JSON value"}
        end

      {:error, reason} ->
        {:error, reason}
    end
  end

  def decode(other), do: {:error, "not a string: #{inspect(other)}"}

  @doc "Object member lookup on a decoded `{:obj, pairs}`."
  @spec get(json(), String.t()) :: {:ok, json()} | :error
  def get({:obj, pairs}, key) do
    case List.keyfind(pairs, key, 0) do
      {^key, value} -> {:ok, value}
      nil -> :error
    end
  end

  def get(_other, _key), do: :error

  @spec encode(json()) :: String.t()
  def encode(value), do: IO.iodata_to_binary(encode_value(value))

  # --- decoding ---------------------------------------------------------------

  defp ws(<<char, rest::binary>>) when char in [?\s, ?\t, ?\n, ?\r], do: ws(rest)
  defp ws(binary), do: binary

  defp parse_value(<<"{", rest::binary>>) do
    case ws(rest) do
      <<"}", rest::binary>> -> {:ok, {:obj, []}, rest}
      binary -> parse_object_members(binary, [])
    end
  end

  defp parse_value(<<"[", rest::binary>>) do
    case ws(rest) do
      <<"]", rest::binary>> -> {:ok, [], rest}
      binary -> parse_array_items(binary, [])
    end
  end

  defp parse_value(<<"\"", rest::binary>>), do: parse_string(rest, [])
  defp parse_value(<<"true", rest::binary>>), do: {:ok, true, rest}
  defp parse_value(<<"false", rest::binary>>), do: {:ok, false, rest}
  defp parse_value(<<"null", rest::binary>>), do: {:ok, nil, rest}

  defp parse_value(<<char, _::binary>> = binary) when char in ~c"-0123456789",
    do: parse_number(binary)

  defp parse_value(<<>>), do: {:error, "unexpected end of input"}
  defp parse_value(<<char, _::binary>>), do: {:error, "unexpected character #{<<char>>}"}

  defp parse_object_members(binary, pairs) do
    with <<"\"", rest::binary>> <- ws(binary),
         {:ok, key, rest} <- parse_string(rest, []),
         <<":", rest::binary>> <- ws(rest),
         {:ok, value, rest} <- parse_value(ws(rest)) do
      case ws(rest) do
        <<",", rest::binary>> -> parse_object_members(ws(rest), [{key, value} | pairs])
        <<"}", rest::binary>> -> {:ok, {:obj, Enum.reverse([{key, value} | pairs])}, rest}
        _ -> {:error, "expected ',' or '}' in object"}
      end
    else
      {:error, reason} -> {:error, reason}
      _ -> {:error, "malformed object member"}
    end
  end

  defp parse_array_items(binary, items) do
    with {:ok, value, rest} <- parse_value(binary) do
      case ws(rest) do
        <<",", rest::binary>> -> parse_array_items(ws(rest), [value | items])
        <<"]", rest::binary>> -> {:ok, Enum.reverse([value | items]), rest}
        _ -> {:error, "expected ',' or ']' in array"}
      end
    end
  end

  defp parse_string(<<"\"", rest::binary>>, acc), do: {:ok, IO.iodata_to_binary(acc), rest}

  defp parse_string(<<"\\", escape, rest::binary>>, acc) do
    case escape do
      ?" -> parse_string(rest, [acc, ?"])
      ?\\ -> parse_string(rest, [acc, ?\\])
      ?/ -> parse_string(rest, [acc, ?/])
      ?b -> parse_string(rest, [acc, ?\b])
      ?f -> parse_string(rest, [acc, ?\f])
      ?n -> parse_string(rest, [acc, ?\n])
      ?r -> parse_string(rest, [acc, ?\r])
      ?t -> parse_string(rest, [acc, ?\t])
      ?u -> unicode_escape(rest, acc)
      _ -> {:error, "invalid escape \\#{<<escape>>}"}
    end
  end

  defp parse_string(<<char, _rest::binary>>, _acc) when char < 0x20,
    do: {:error, "unescaped control character in string"}

  defp parse_string(<<char, rest::binary>>, acc), do: parse_string(rest, [acc, char])
  defp parse_string(<<>>, _acc), do: {:error, "unterminated string"}

  defp unicode_escape(rest, acc) do
    with {:ok, high, rest} <- hex4(rest) do
      cond do
        high in 0xD800..0xDBFF ->
          case rest do
            <<"\\u", tail::binary>> ->
              with {:ok, low, rest} <- hex4(tail),
                   true <- low in 0xDC00..0xDFFF do
                codepoint = 0x10000 + (high - 0xD800) * 0x400 + (low - 0xDC00)
                parse_string(rest, [acc, <<codepoint::utf8>>])
              else
                {:error, reason} -> {:error, reason}
                _ -> {:error, "unpaired high surrogate"}
              end

            _ ->
              {:error, "unpaired high surrogate"}
          end

        high in 0xDC00..0xDFFF ->
          {:error, "unpaired low surrogate"}

        true ->
          parse_string(rest, [acc, <<high::utf8>>])
      end
    end
  end

  defp hex4(<<a, b, c, d, rest::binary>>) do
    digits = [a, b, c, d]

    if Enum.all?(digits, &(not is_nil(hex_digit(&1)))) do
      value =
        digits |> Enum.map(&hex_digit/1) |> Enum.reduce(0, fn digit, acc -> acc * 16 + digit end)

      {:ok, value, rest}
    else
      {:error, "invalid \\u escape"}
    end
  end

  defp hex4(_binary), do: {:error, "unterminated \\u escape"}

  defp hex_digit(char) when char in ?0..?9, do: char - ?0
  defp hex_digit(char) when char in ?a..?f, do: char - ?a + 10
  defp hex_digit(char) when char in ?A..?F, do: char - ?A + 10
  defp hex_digit(_char), do: nil

  defp parse_number(binary), do: parse_number(binary, [])

  defp parse_number(<<char, rest::binary>>, acc) when char in ~c"+-0123456789.eE",
    do: parse_number(rest, [acc, char])

  defp parse_number(rest, acc) do
    text = IO.iodata_to_binary(acc)

    cond do
      text == "" ->
        {:error, "expected a number"}

      String.contains?(text, [".", "e", "E"]) ->
        case Float.parse(text) do
          {float, ""} -> {:ok, float, rest}
          _ -> {:error, "invalid number: #{text}"}
        end

      true ->
        case Integer.parse(text) do
          {int, ""} -> {:ok, int, rest}
          _ -> {:error, "invalid number: #{text}"}
        end
    end
  end

  # --- encoding ---------------------------------------------------------------

  defp encode_value(nil), do: "null"
  defp encode_value(true), do: "true"
  defp encode_value(false), do: "false"
  defp encode_value(int) when is_integer(int), do: Integer.to_string(int)
  defp encode_value(float) when is_float(float), do: :erlang.float_to_binary(float, [:short])
  defp encode_value(string) when is_binary(string), do: encode_string(string)
  defp encode_value({:obj, pairs}), do: encode_object(pairs)

  defp encode_value(items) when is_list(items) do
    members = Enum.intersperse(Enum.map(items, &encode_value/1), ?,)
    IO.iodata_to_binary([?[ | members ++ [?]]])
  end

  defp encode_object(pairs) do
    members =
      pairs
      |> Enum.map(fn {key, val} -> [encode_string(key), ?:, encode_value(val)] end)
      |> Enum.intersperse(?,)

    IO.iodata_to_binary([?{, members, ?}])
  end

  # Minimal escaping, matching serde_json: only quotes, backslashes, and
  # control characters are escaped; forward slashes pass through.
  defp encode_string(string) do
    escaped = escape_rest(string, [])
    IO.iodata_to_binary([?", escaped, ?"])
  end

  defp escape_rest(<<>>, acc), do: acc

  for {char, text} <- [
        {?", "\\\""},
        {?\\, "\\\\"},
        {?\b, "\\b"},
        {?\f, "\\f"},
        {?\n, "\\n"},
        {?\r, "\\r"},
        {?\t, "\\t"}
      ] do
    defp escape_rest(<<unquote(char), rest::binary>>, acc),
      do: escape_rest(rest, [acc, unquote(text)])
  end

  defp escape_rest(<<codepoint::utf8, rest::binary>>, acc) when codepoint < 0x20 do
    text = "\\u" <> (codepoint |> Integer.to_string(16) |> String.pad_leading(4, "0"))
    escape_rest(rest, [acc, text])
  end

  defp escape_rest(<<codepoint::utf8, rest::binary>>, acc),
    do: escape_rest(rest, [acc, <<codepoint::utf8>>])
end
