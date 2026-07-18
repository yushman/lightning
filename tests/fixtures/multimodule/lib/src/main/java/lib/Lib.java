package lib;

import core.Core;

public class Lib {
    public String value() {
        return "lib:" + new Core().value();
    }
}
