import { Component } from "/classic/ecs.js"


class Animator extends Component {
    constructor(entity, target = null) {
        super(entity);
        this.animation = null;
        this.counter = 0.0;
        this.frame = 0;
        this.repeat = false;
        this._playing = false;

        this.target = target;

        entity.registerCall("update", this.update.bind(this));
    }

    update() {
        if (this._playing || this.repeat) {
            this.counter += this.game.deltaTime * (this.animation.rate);
            let intCounter = Math.floor(this.counter);
            if (intCounter >= this.animation.sequence.length) {
                intCounter = 0;
                this.counter = 0;
                this._playing = false;
            }

            this.frame = this.animation.sequence[intCounter];

            if (this.target != null)
                this.target.frame = this.frame;

        }

    }

    play(animation, repeat = false) {
        this.repeat = repeat;
        this._playing = true;

        this.animation = animation;
    }

};

export { Animator };
