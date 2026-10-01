"""Hand-written AST constructors shared by the corpus builders.

Extracted verbatim from `build_dev_seed.py` so a second corpus can be written
without touching the builder of the frozen `dev-seed-v1` / `dev-seed-v2`
files. These are plain dictionaries: nothing here imports, calls or shells out
to sciwhisper-core, so no gold answer can be a copy of the parser's output.
"""

# --------------------------------------------------------------- chemistry

def atom(symbol, count=1):
    return {"Atom": {"symbol": symbol, "count": count}}

def group(parts, count):
    return {"Group": {"inner": {"parts": parts}, "count": count}}

def hydrate(count):
    return {"Hydrate": {"count": count}}

def formula(*parts):
    return {"parts": list(parts)}

def species(f, coefficient=1, charge=None, marker=None):
    return {"coefficient": coefficient, "formula": f, "charge": charge, "marker": marker}

def chem(s):
    return {"Chemical": {"Species": s}}

def reaction(left, right, arrow="Forward", condition=None):
    return {"Chemical": {"Equation": {"left": left, "arrow": arrow,
                                      "right": right, "condition": condition}}}

# ------------------------------------------------------------- mathematics

def math(node):
    return {"Math": node}

def num(text):
    return {"Number": text}

def sym(letter, case="Lower"):
    return {"Symbol": {"letter": letter, "alphabet": "Latin", "case": case}}

def greek(letter, case="Lower"):
    return {"Symbol": {"letter": letter, "alphabet": "Greek", "case": case}}

def binary(op, left, right):
    return {"Binary": {"op": op, "left": left, "right": right}}

def juxt(*items):
    return {"Juxt": list(items)}

def fraction(numerator, denominator):
    return {"Fraction": {"num": numerator, "den": denominator}}

def power(base, exponent):
    return {"Power": {"base": base, "exp": exponent}}

def subscript(base, sub):
    return {"Subscript": {"base": base, "sub": sub}}

def root(radicand, index=None):
    return {"Root": {"index": index, "radicand": radicand}}

def paren(inner):
    return {"Group": {"kind": "Paren", "inner": inner}}

def fn(kind, arg):
    return {"Function": {"kind": kind, "arg": arg}}

def unit(*factors):
    return {"Unit": {"factors": [{"symbol": s, "power": p, "divide": d} for s, p, d in factors]}}

def quantity(value, *factors):
    return juxt(num(value), unit(*factors))

def neg(inner):
    return {"UnaryMinus": inner}

def delta(inner):
    return {"Delta": inner}
