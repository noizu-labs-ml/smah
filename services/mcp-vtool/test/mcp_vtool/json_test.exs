defmodule McpVtool.JsonTest do
  use ExUnit.Case, async: true

  alias McpVtool.Json

  describe "decode" do
    test "preserves object key order as written" do
      assert {:ok, {:obj, [{"b", 1}, {"a", "x"}]}} = Json.decode(~s({"b": 1, "a": "x"}))
    end

    test "scalar values" do
      assert {:ok, 42} = Json.decode("42")
      assert {:ok, -7} = Json.decode(" -7 ")
      assert {:ok, 1.5} = Json.decode("1.5")
      assert {:ok, true} = Json.decode("true")
      assert {:ok, false} = Json.decode("false")
      assert {:ok, nil} = Json.decode("null")
    end

    test "empty containers stay distinct" do
      assert {:ok, {:obj, []}} = Json.decode("{}")
      assert {:ok, []} = Json.decode("[]")
    end

    test "nested structures" do
      text = ~s({"list": [1, {"k": "v"}], "deep": {"x": [true, null]}})
      assert {:ok, decoded} = Json.decode(text)
      assert {:ok, "v"} = Json.get(Json.get(decoded, "list") |> elem(1) |> Enum.at(1), "k")
      assert {:ok, deep} = Json.get(decoded, "deep")
      assert {:ok, [true, nil]} = Json.get(deep, "x")
    end

    test "string escapes and unicode" do
      assert {:ok, "a\"b\nc\\d"} = Json.decode(~s("a\\"b\\nc\\\\d"))
      assert {:ok, "Aé😀"} = Json.decode(~s("\\u0041\\u00e9\\ud83d\\ude00"))
      assert {:ok, "tab\there"} = Json.decode(~s("tab\\there"))
    end

    test "malformed input errors" do
      assert {:error, _} = Json.decode("")
      assert {:error, _} = Json.decode("{")
      assert {:error, _} = Json.decode("[1,]")
      assert {:error, _} = Json.decode("{\"a\" 1}")
      assert {:error, _} = Json.decode("1 2")
      assert {:error, _} = Json.decode("nul")
      assert {:error, _} = Json.decode(~s("unterminated))
      assert {:error, _} = Json.decode(~s("\\ud83d no low"))
    end
  end

  describe "encode" do
    test "objects keep insertion order (serde canonical form)" do
      assert Json.encode({:obj, [{"b", 1}, {"a", "x"}]}) == ~s({"b":1,"a":"x"})
    end

    test "empty containers" do
      assert Json.encode({:obj, []}) == "{}"
      assert Json.encode([]) == "[]"
    end

    test "escapes match serde minimal form" do
      assert Json.encode("a\"b\nc\\d") == ~s("a\\"b\\nc\\\\d")
      assert Json.encode("slash/not/escaped") == ~s("slash/not/escaped")
    end

    test "numbers and nested" do
      assert Json.encode(42) == "42"
      assert Json.encode(-7) == "-7"
      assert Json.encode(1.5) == "1.5"

      assert Json.encode({:obj, [{"list", [1, {:obj, [{"k", "v"}]}]}]}) ==
               ~s({"list":[1,{"k":"v"}]})
    end

    test "decode then encode is the serde canonical round trip" do
      text = ~s({"q":"ada","limit":2})
      assert {:ok, args} = Json.decode(text)
      assert Json.encode(args) == text
    end
  end
end
