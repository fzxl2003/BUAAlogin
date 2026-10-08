"""Container entry: credentials are mandatory even with an attached terminal."""
import sys
from always_online import main

if __name__ == '__main__':
    sys.exit(main(require_credentials=True, try_all_interfaces=True))
