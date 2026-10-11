import ast
import textwrap
from enum import Enum
from typing import Optional

from lxml import etree

from smah.console import err_console


class ConditionError(ValueError):
    """
    An exec-if condition cannot be evaluated safely: unknown variable, unsafe
    construct (attribute access, calls, subscripts...), or a type mismatch.
    """


def _compare(op: ast.cmpop, left, right) -> bool:
    try:
        if isinstance(op, ast.Eq):
            return left == right
        if isinstance(op, ast.NotEq):
            return left != right
        if isinstance(op, ast.Lt):
            return left < right
        if isinstance(op, ast.LtE):
            return left <= right
        if isinstance(op, ast.Gt):
            return left > right
        if isinstance(op, ast.GtE):
            return left >= right
        if isinstance(op, ast.In):
            return left in right
        if isinstance(op, ast.NotIn):
            return left not in right
    except TypeError as exc:
        raise ConditionError(f"cannot compare {left!r} with {right!r}") from exc
    raise ConditionError("unsupported comparison operator")


def _eval_condition_node(node: ast.AST, names: dict):
    """Walk a whitelisted expression AST. Anything else raises ConditionError."""
    if isinstance(node, ast.Expression):
        return _eval_condition_node(node.body, names)
    if isinstance(node, ast.Constant):
        return node.value
    if isinstance(node, ast.Name):
        # Models emit JSON-style literals (true/false/null) that are not
        # Python constants; normalize them instead of failing the condition.
        if node.id in names:
            return names[node.id]
        if node.id == "true":
            return True
        if node.id == "false":
            return False
        if node.id == "null":
            return None
        raise ConditionError(f"unknown condition variable '{node.id}'")
    if isinstance(node, ast.BoolOp):
        if isinstance(node.op, ast.And):
            for value in node.values:
                if not _eval_condition_node(value, names):
                    return False
            return True
        if isinstance(node.op, ast.Or):
            for value in node.values:
                if _eval_condition_node(value, names):
                    return True
            return False
        raise ConditionError("unsupported boolean operator")
    if isinstance(node, ast.UnaryOp):
        if isinstance(node.op, ast.Not):
            return not _eval_condition_node(node.operand, names)
        raise ConditionError("unsupported unary operator")
    if isinstance(node, ast.Compare):
        left = _eval_condition_node(node.left, names)
        for op, comparator in zip(node.ops, node.comparators):
            right = _eval_condition_node(comparator, names)
            if not _compare(op, left, right):
                return False
            left = right
        return True
    raise ConditionError(f"unsupported expression node: {type(node).__name__}")

html_tags = [
    "a",
    "abbr",
    "acronym",
    "address",
    "applet",
    "area",
    "article",
    "aside",
    "audio",
    "b",
    "base",
    "basefont",
    "bdo",
    "big",
    "blockquote",
    "body",
    "br",
    "button",
    "canvas",
    "caption",
    "center",
    "cite",
    "code",
    "col",
    "colgroup",
    "datalist",
    "dd",
    "del",
    "dfn",
    "div",
    "dl",
    "dt",
    "em",
    "embed",
    "fieldset",
    "figcaption",
    "figure",
    "font",
    "footer",
    "form",
    "frame",
    "frameset",
    "head",
    "header",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "hr",
    "html",
    "i",
    "iframe",
    "img",
    "input",
    "ins",
    "kbd",
    "label",
    "legend",
    "li",
    "link",
    "main",
    "map",
    "mark",
    "meta",
    "meter",
    "nav",
    "noscript",
    "object",
    "ol",
    "optgroup",
    "option",
    "p",
    "param",
    "pre",
    "progress",
    "q",
    "s",
    "samp",
    "script",
    "section",
    "select",
    "small",
    "source",
    "span",
    "strike",
    "strong",
    "style",
    "sub",
    "sup",
    "table",
    "tbody",
    "td",
    "textarea",
    "tfoot",
    "th",
    "thead",
    "time",
    "title",
    "tr",
    "u",
    "ul",
    "var",
    "video",
    "wbr",
]

class ThoughtType(Enum):
    OTHER = 0
    THINKING = 1
    QUESTION = 2
    ASSUMPTION = 3
    INNER_CRITIC = 4
    TANGENT = 5

class TagBase(etree.ElementBase):
    def extract_child(self, tag: str):
        child = self.cssselect(tag)
        if len(child) > 0:
            child = child[0]
            return ResponseParser.unescape_response(child.text.strip())
        return None

class SetConditionTag(TagBase):
    @property
    def name(self):
        return ResponseParser.unescape_response(self.get("name"))

    @property
    def prompt(self):
        prompt = self.extract_child("prompt")
        if prompt:
            return prompt
        else:
            name = ResponseParser.unescape_response(self.get("name"))
            return f"Set {name}"

    @property
    def choices(self):
        choices = []
        x = self.cssselect("choices choice")
        if len(x) > 0:
            for c in x:
                c_value = ResponseParser.unescape_response(c.get("value"))
                c_label = ResponseParser.unescape_response(c.text or c_value)
                i = None
                c_user = c.get("data-user") == "true"
                if c_user:
                    i = {
                        'prompt': ResponseParser.unescape_response(c.get("data-prompt") or f"Enter {c_label}"),
                        'required': c.get("data-required") == "true",
                        'check': ResponseParser.unescape_response(c.get("data-check"))
                    }
                choices.append({
                    'value': c_value,
                    'label': c_label,
                    'prompt': i
                })
        return choices








class ThoughtTag(TagBase):
    THOUGHT_TYPES = {
        "tangent": ThoughtType.TANGENT,
        "thinking": ThoughtType.THINKING,
        "question": ThoughtType.QUESTION,
        "assumption": ThoughtType.ASSUMPTION,
        "inner-critic": ThoughtType.INNER_CRITIC
    }


    def _init(self):
        self.tag = "cot"

    @property
    def type(self):
        return self.THOUGHT_TYPES.get(self.get("type"), ThoughtType.OTHER)

    @property
    def thought(self):
        return ResponseParser.unescape_response(self.text)

    @property
    def markdown(self):
        type = self.THOUGHT_TYPES.get(self.get("type"), ThoughtType.OTHER)
        if type == ThoughtType.THINKING:
            type = "Thinking"
        elif type == ThoughtType.QUESTION:
            type = "Question"
        elif type == ThoughtType.ASSUMPTION:
            type = "Assumption"
        elif type == ThoughtType.INNER_CRITIC:
            type = "Inner-Critic"
        elif type == ThoughtType.TANGENT:
            type = "Tangent"
        else:
            type = "Other"
        body = self.text or ""
        body = ResponseParser.unescape_response(body.replace("`","'"))
        return f"`{type}: {body}`"

class ExecTag(TagBase):
    def _init(self):
        self.tag = "exec"

    @property
    def shell(self):
        return self.get("shell")

    @property
    def exec_if(self):
        return ResponseParser.unescape_response(self.get("exec-if"))

    @property
    def title(self):
        return self.extract_child("title") or "Run Command"

    @property
    def purpose(self):
        return self.extract_child("purpose") or "No Purpose Provided"

    @property
    def command(self):
        return self.extract_child("command")

    @property
    def markdown(self):
        title = self.extract_child("title") or "Run Command"
        title = textwrap.indent(title, "# ")
        shell = self.get("shell")
        command = self.extract_child("command") or ""

        template = textwrap.dedent(
            """
            ```{shell}
            {title}
            
            {command}
            ```
            """
        ).strip().format(
            title=title,
            shell=shell,
            command=command
        )
        return template


class SmahLookup(etree.CustomElementClassLookup):
    def lookup(self, node_type, document, namespace, name):
        if node_type == "element":
            if name == "exec":
                return ExecTag
            elif name == "cot":
                return ThoughtTag
            elif name == "set-condition":
                return SetConditionTag
        return None

class ResponseParser:
    def __init__(self):
        pass

    @staticmethod
    def replace_tag(elem: any, replace: Optional[str] = None, tail: Optional[str] = None):
        ps = elem.getprevious()
        p = elem.getparent()
        if ps is not None:
            t = (ps.tail or "")
            l = t.split("\n")
            if len(l) > 0:
                r = len(l[-1])
                if replace:
                    t += textwrap.indent(replace, " " * r).lstrip()
                t += (tail or "")
            else:
                t += ((replace or "").lstrip() + (tail or ""))
            ps.tail = t

        elif p is not None:
            t = (p.text or "")
            l = t.split("\n")
            if len(l) > 0:
                r = len(l[-1])
                if replace:
                    t += textwrap.indent(replace, " " * r).lstrip()
                t += (tail or "")
            else:
                t += ((replace or "").lstrip() + (tail or ""))
            p.text = t
        if p is not None:
            p.remove(elem)


    @staticmethod
    def extract_conditions(response: str, options: Optional[dict] = None) -> Optional[list]:
        options = options or {}
        parser = etree.XMLPullParser(recover=True)
        parser.set_element_class_lookup(SmahLookup())
        events = parser.read_events()
        response = ResponseParser.escape_response(response)
        parser.feed("<smah-msg>" + response + "</smah-msg>")
        response = []
        for action, elem in events:
            if action == "end":
                if isinstance(elem, SetConditionTag):
                    c = {
                        'name': ResponseParser.unescape_response(elem.name),
                        'prompt': ResponseParser.unescape_response(elem.prompt),
                        'choices': elem.choices
                    }
                    response.append(c)
        return response

    @staticmethod
    def extract_commands(response: str, options: Optional[dict] = None) -> Optional[list]:
        options = options or {}
        parser = etree.XMLPullParser(recover=True)
        parser.set_element_class_lookup(SmahLookup())
        events = parser.read_events()

        response = ResponseParser.escape_response(response)
        parser.feed("<smah-msg>" + response + "</smah-msg>")
        commands = []
        for action, elem in events:
            if action == "end":
                if isinstance(elem, ExecTag):
                    conditions = options.get("conditions") or {}
                    # include operator and system
                    condition = elem.exec_if  # already unescaped by the property
                    include = True
                    skip_reason = None
                    if condition is not None:
                        try:
                            include = ResponseParser.eval_condition(condition, conditions)
                            if not include:
                                skip_reason = f"falsy condition: {condition}"
                        except ConditionError as exc:
                            include = False
                            skip_reason = f"unevaluable condition: {condition} ({exc})"
                    if include:
                        c = {
                            'title': ResponseParser.unescape_response(elem.title),
                            'purpose': ResponseParser.unescape_response(elem.purpose),
                            'command': ResponseParser.unescape_response(elem.command),
                            'shell': ResponseParser.unescape_response(elem.shell)
                        }
                        commands.append(c)
                    else:
                        err_console.print(f"Skipping command: {elem.title} due to {skip_reason}")
        return commands

    @staticmethod
    def eval_condition(condition: str, names: Optional[dict] = None) -> bool:
        """
        Safely evaluate an exec-if condition written by the model.

        The condition is parsed with `ast` and walked over a strict whitelist
        of expression nodes: constants, variable names, not/and/or, and the
        comparisons ==/!=/</<=/>/>=/in/not in. There is no eval()/exec() on
        model output anywhere: unknown names, attribute access, calls,
        subscripts, and any other construct raise ConditionError instead of
        executing. Callers treat an unevaluable condition as "skip command".
        """
        names = names or {}
        try:
            tree = ast.parse(condition, mode="eval")
        except SyntaxError as exc:
            raise ConditionError(f"not a valid expression: {condition}") from exc
        return bool(_eval_condition_node(tree, names))

    @staticmethod
    def extract_json(response: str) -> str:
        """
        Best-effort extraction of a JSON object from a model response: strips
        markdown code fences and any prose around the outermost {...} pair.
        A response that already is a bare JSON object passes through unchanged.
        """
        text = (response or "").strip()
        if text.startswith("```"):
            newline = text.find("\n")
            text = text[newline + 1:] if newline != -1 else ""
            if text.rstrip().endswith("```"):
                text = text.rstrip()[:-3]
            text = text.strip()
        if not text.startswith("{"):
            start = text.find("{")
            end = text.rfind("}")
            if start != -1 and end != -1 and end > start:
                text = text[start:end + 1]
        return text

    @staticmethod
    def escape_response(response: str) -> str:
        response = response.replace("<", ":_smah_lt_:")
        response = response.replace("&amp;", ":_smah_amp_amp_:")
        response = response.replace("&", ":_smah_amp_:")

        for tag in ["smah-", "cot", "exec", "prompt", "title", "command", "set-condition", "choices", "choice"]:
            response = response.replace(f":_smah_lt_:{tag}", f"<{tag}")
            response = response.replace(f":_smah_lt_:/{tag}", f"</{tag}")

        for tag in html_tags:
            response = response.replace(f":_smah_lt_:{tag}", f"<{tag}")
            response = response.replace(f":_smah_lt_:/{tag}", f"</{tag}")


        return response

    @staticmethod
    def unescape_response(response: Optional[str]) -> Optional[str]:
        if response is None:
            return None
        response = response.replace(":_smah_lt_:", "<")
        response = response.replace(":_smah_amp_amp_:", "&amp;")
        response = response.replace(":_smah_amp_:", "&")

        return response

    @staticmethod
    def to_markdown(response: str, options: Optional[dict] = None) -> str:
        options = options or {}
        parser = etree.XMLPullParser(recover=True, encoding="utf-8")
        parser.set_element_class_lookup(SmahLookup())
        events = parser.read_events()
        response = ResponseParser.escape_response(response)



        parser.feed("<smah-msg>\n" + response + "\n</smah-msg>")
        for action, elem in events:
            if action == "end":
                if isinstance(elem, ExecTag):
                    ResponseParser.replace_tag(elem, replace=elem.markdown, tail=elem.tail)
                elif isinstance(elem, SetConditionTag):
                    ResponseParser.replace_tag(elem, replace=None, tail=elem.tail)
                elif isinstance(elem, ThoughtTag):
                    ResponseParser.replace_tag(elem, replace=(None if options.get("strip-cot", True) else elem.markdown), tail=elem.tail)

        root = parser.close()
        response = etree.tostring(root, pretty_print=True).decode()
        response = ResponseParser.unescape_response(response)


        return response[len("<smah-msg>\n"):-(len("\n</smah-msg>") + 1)]



